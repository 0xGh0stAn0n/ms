<#
.SYNOPSIS
  Measures the live call the way the phone uses it: recorded questions are
  spoken into the call's own session, in real time, and each turn is timed
  from the end of the question to the first sound of the answer.

.DESCRIPTION
  The call is MapleSyrup's own: the session comes from the running
  MapleSyrup (/api/live: its model, instructions, tools, turn detection and
  reasoning), and the tools the call asks for run there too (/api/tool), the
  thinker included. Only the microphone and the speaker are stand-ins: the
  questions are synthesized once (OpenAI's voice, with MapleSyrup's key,
  which stays on this PC) and streamed as 24 kHz PCM over a WebSocket, with
  room silence after them, as the phone streams its microphone.

  Per turn it writes when the speech ended (the last chunk sent), when the
  model's turn detection heard it stop, when the response began, its first
  sound, what it said, and the tools it ran (a tool's answer is timed again,
  from the tool's output to the next first sound). A summary and every
  event go to bench\results beside MapleSyrup.

  Starts MapleSyrup (no panel, no voice, no update) when it isn't running,
  and stops it again at the end.

.PARAMETER Questions
  A file of questions, one per line (default: questions.txt beside this
  script, else the built-in set).

.PARAMETER Effort
  Override the call's reasoning effort for this run (minimal, low, medium,
  high), to compare.

.PARAMETER Eagerness
  Override semantic VAD's eagerness for this run (low, medium, high, auto).

.PARAMETER Mic
  Where the call is told the microphone is: far (a PC's, the default) or
  near (a phone held close); it decides how the call cleans noise.

.PARAMETER Lang
  The phone's language the call is set to (default he-IL): every answer is
  checked to be in it.

.NOTES
  Besides the questions it plays a room with nobody talking (does the call
  answer noise?) and talks over a long answer (how soon does it notice?).
  Tools that would change the player's settings or notebook are recorded
  and not run: a bench never moves MapleSyrup's settings.

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File bench_live.ps1
#>
param(
    [string]$Questions = "",
    [string]$Effort = "",
    [string]$Eagerness = "",
    [string]$Label = "",
    [string]$Mic = "far",
    [string]$Lang = "he-IL",
    [int]$Port = 8443,
    [string]$TtsVoice = "ash"
)

$ErrorActionPreference = "Stop"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

# ---- where things are ---------------------------------------------------
$Here = Split-Path -Parent $MyInvocation.MyCommand.Path
$Settings = Join-Path $env:APPDATA "MapleSyrup"
$AppDir = Split-Path -Parent $Here
$Exe = Join-Path $AppDir "MapleSyrup.exe"
$AudioDir = Join-Path $Here "audio"
$ResultsDir = Join-Path $Here "results"
New-Item -ItemType Directory -Force -Path $AudioDir, $ResultsDir | Out-Null

# Milliseconds of audio per chunk, as the phone sends its microphone; room
# silence before a question; how loud a sample must be to count as speech
# (the voice pads its end with silence, which is not the player still
# talking: a question ends at its last sound, plus a little); the most one
# turn may take.
$ChunkMs = 20
$LeadSilenceMs = 300
$SpeechFloor = 328
$SpeechTailMs = 30
$TurnMaxMs = 30000
$QuietBetweenMs = 1500
$SampleRate = 24000
# The room with nobody talking: how long, how loud (RMS, of full scale:
# 0.01 is -40 dB), and how long to listen after it for an answer.
$NoiseMs = 4000
$NoiseRms = 0.01
$AfterNoiseMs = 4000
# Talking over it: how long into its answer.
$OverAfterMs = 1200
# The tools a bench may run: they only look. The rest would change the
# player's settings or notebook, and are recorded instead.
$ToolsRun = @("think_it_through", "look_closer", "look_it_up")

# The built-in questions: chit-chat, a number off the reading, things to work
# out, in Hebrew and English, short and long.
$BuiltIn = @(
    "היי, מה קורה?",
    "כמה HP יש לי עכשיו?",
    "כמה זמן ייקח לי להגיע לרמה שלושים בקצב הזה?",
    "Hey, how's it going?",
    "What should I do next?",
    "אני חושב לעבור למפה אחרת כי פה יש יותר מדי שחקנים, יש לך רעיון לאן כדאי לי ללכת עם הרמה שלי?"
)
# A question with a long answer, and what is said over it.
$LongQuestion = "תספר לי בפירוט את כל הסיפור של מייפל איילנד מההתחלה"
$OverQuestion = "רגע רגע, עצור. כמה MP יש לי?"
# A sentence with a pause in its middle, as a player thinking aloud: does
# it jump in during the pause?
$PauseFirst = "אני חושב שאולי כדאי לי"
$PauseSecond = "לעבור למפה אחרת, מה אתה אומר?"
$PauseMs = 900

# ---- a WebSocket with timestamps, and the localhost certificate ---------
Add-Type -TypeDefinition @"
using System;
using System.Collections.Concurrent;
using System.Diagnostics;
using System.IO;
using System.Net;
using System.Net.Security;
using System.Net.WebSockets;
using System.Security.Cryptography.X509Certificates;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

public static class BenchClock {
    static readonly Stopwatch Watch = Stopwatch.StartNew();
    public static double Ms() { return Watch.Elapsed.TotalMilliseconds; }
}

public static class BenchTls {
    // MapleSyrup's phone link has a certificate it made itself: trusted for
    // this PC's own address only; everything else is checked as usual.
    public static void TrustLocalLink() {
        ServicePointManager.ServerCertificateValidationCallback = Check;
    }
    static bool Check(object sender, X509Certificate cert, X509Chain chain, SslPolicyErrors errors) {
        if (errors == SslPolicyErrors.None) return true;
        var request = sender as HttpWebRequest;
        return request != null && (request.Address.Host == "127.0.0.1" || request.Address.Host == "localhost");
    }
}

public static class BenchAudio {
    // A room's noise, `ms` long at `rms` of full scale: white noise smoothed
    // to a hum, 16-bit PCM.
    public static byte[] Noise(int rate, int ms, double rms, int seed) {
        var random = new Random(seed);
        int n = rate * ms / 1000;
        var values = new double[n];
        double smooth = 0, sum = 0;
        for (int i = 0; i < n; i++) {
            smooth = 0.85 * smooth + 0.15 * (random.NextDouble() * 2 - 1);
            values[i] = smooth;
            sum += smooth * smooth;
        }
        double scale = rms / Math.Sqrt(sum / Math.Max(1, n)) * 32767;
        var pcm = new byte[n * 2];
        for (int i = 0; i < n; i++) {
            int v = (int)Math.Max(-32768, Math.Min(32767, Math.Round(values[i] * scale)));
            pcm[2 * i] = (byte)(v & 0xff);
            pcm[2 * i + 1] = (byte)((v >> 8) & 0xff);
        }
        return pcm;
    }

    // Where the speech in 16-bit PCM ends: `tailBytes` after the last sample
    // louder than `floor`.
    public static int SpeechEnd(byte[] pcm, int floor, int tailBytes) {
        for (int i = pcm.Length - 2; i >= 0; i -= 2) {
            int sample = (short)(pcm[i] | (pcm[i + 1] << 8));
            if (Math.Abs(sample) > floor) return Math.Min(pcm.Length, i + 2 + tailBytes);
        }
        return pcm.Length;
    }
}

public class BenchEvent { public double At; public string Json; }

public class BenchSocket {
    readonly ClientWebSocket socket = new ClientWebSocket();
    readonly object sending = new object();
    public readonly ConcurrentQueue<BenchEvent> Events = new ConcurrentQueue<BenchEvent>();
    public volatile int Done;
    public volatile int Sounds;
    public volatile string Error;

    public void Connect(string url, string bearer) {
        socket.Options.SetRequestHeader("Authorization", "Bearer " + bearer);
        socket.ConnectAsync(new Uri(url), CancellationToken.None).Wait();
        Task.Run(() => Receive());
    }

    async Task Receive() {
        var buffer = new byte[1 << 16];
        var message = new MemoryStream();
        try {
            while (socket.State == WebSocketState.Open) {
                var got = await socket.ReceiveAsync(new ArraySegment<byte>(buffer), CancellationToken.None);
                if (got.MessageType == WebSocketMessageType.Close) break;
                message.Write(buffer, 0, got.Count);
                if (!got.EndOfMessage) continue;
                var text = Encoding.UTF8.GetString(message.ToArray());
                message.SetLength(0);
                Events.Enqueue(new BenchEvent { At = BenchClock.Ms(), Json = text });
                if (text.Contains("\"type\":\"response.done\"")) Done++;
                if (text.Contains("\"type\":\"response.output_audio.delta\"") || text.Contains("\"type\":\"response.audio.delta\"")) Sounds++;
            }
        } catch (Exception e) { Error = e.Message; }
    }

    public void Send(string json) {
        var bytes = Encoding.UTF8.GetBytes(json);
        lock (sending) {
            socket.SendAsync(new ArraySegment<byte>(bytes), WebSocketMessageType.Text, true, CancellationToken.None).Wait();
        }
    }

    void Chunk(byte[] pcm, int offset, int length) {
        Send("{\"type\":\"input_audio_buffer.append\",\"audio\":\"" + Convert.ToBase64String(pcm, offset, length) + "\"}");
    }

    // PCM16 mono at `rate`, in real time, `chunkMs` a chunk. Returns when
    // the last chunk went (the end of the speech).
    public double Stream(byte[] pcm, int rate, int chunkMs) {
        int size = rate * 2 * chunkMs / 1000;
        double start = BenchClock.Ms();
        int n = 0;
        for (int offset = 0; offset < pcm.Length; offset += size, n++) {
            double due = start + n * chunkMs;
            while (BenchClock.Ms() < due) Thread.Sleep(1);
            Chunk(pcm, offset, Math.Min(size, pcm.Length - offset));
        }
        return BenchClock.Ms();
    }

    // Room silence in real time until `doneBefore` responses more have
    // finished, or `maxMs` passed.
    public void Silence(int rate, int chunkMs, int doneBefore, int moreDone, int maxMs) {
        var silence = new byte[rate * 2 * chunkMs / 1000];
        double start = BenchClock.Ms();
        int n = 0;
        while (Done < doneBefore + moreDone && BenchClock.Ms() - start < maxMs && Error == null) {
            double due = start + n * chunkMs;
            while (BenchClock.Ms() < due) Thread.Sleep(1);
            Chunk(silence, 0, silence.Length);
            n++;
        }
    }

    // Room silence in real time for `ms`, or until a sound more than
    // `soundsBefore` came (`untilSound`).
    public void Quiet(int rate, int chunkMs, int ms, bool untilSound, int soundsBefore) {
        var silence = new byte[rate * 2 * chunkMs / 1000];
        double start = BenchClock.Ms();
        int n = 0;
        while (BenchClock.Ms() - start < ms && Error == null && !(untilSound && Sounds > soundsBefore)) {
            double due = start + n * chunkMs;
            while (BenchClock.Ms() < due) Thread.Sleep(1);
            Chunk(silence, 0, silence.Length);
            n++;
        }
    }

    public void Close() {
        try { socket.CloseAsync(WebSocketCloseStatus.NormalClosure, "", CancellationToken.None).Wait(2000); } catch { }
    }
}
"@

[BenchTls]::TrustLocalLink()

# ---- MapleSyrup: running, or started for the run --------------------------
# A key as MapleSyrup saves it (a byte-order mark, quotes or lines after it
# are not the key).
function Read-Key([string]$File) {
    $text = (Get-Content -Raw -Encoding UTF8 $File).TrimStart([char]0xFEFF).Trim()
    return ($text -split "`r?`n")[0].Trim().Trim([char]34, [char]39).Trim()
}
$LinkKey = Read-Key (Join-Path $Settings "link-key.txt")
$Base = "https://127.0.0.1:$Port"

function Invoke-Link([string]$Method, [string]$Path, $Body) {
    $uri = "$Base$Path" + $(if ($Path.Contains("?")) { "&" } else { "?" }) + "k=$LinkKey"
    $json = if ($null -ne $Body) { $Body | ConvertTo-Json -Depth 10 -Compress } else { $null }
    if ($json) {
        $bytes = [Text.Encoding]::UTF8.GetBytes($json)
        return Invoke-RestMethod -Method $Method -Uri $uri -Body $bytes -ContentType "application/json; charset=utf-8" -TimeoutSec 90
    }
    return Invoke-RestMethod -Method $Method -Uri $uri -TimeoutSec 90
}

function Test-Link {
    try { Invoke-Link "GET" "/api/state" $null | Out-Null; return $true } catch { return $false }
}

$Started = $null
if (-not (Test-Link)) {
    Write-Host "Starting MapleSyrup for the run..."
    $Started = Start-Process -FilePath $Exe -ArgumentList "--no-overlay", "--no-voice", "--plain", "--no-update", "--latency" -WorkingDirectory $AppDir -WindowStyle Minimized -PassThru
    $deadline = (Get-Date).AddSeconds(60)
    while (-not (Test-Link)) {
        if ((Get-Date) -gt $deadline) { throw "MapleSyrup's phone link did not come up on port $Port" }
        Start-Sleep -Milliseconds 500
    }
}

# Which MapleSyrup is measured: its program and when it was built (the
# installed one if it was running, else the one beside this bench).
$Measured = try {
    $proc = Get-Process -Name "MapleSyrup" -ErrorAction Stop | Select-Object -First 1
    "{0} ({1})" -f $proc.Path, (Get-Item $proc.Path).LastWriteTime.ToString("yyyy-MM-dd HH:mm")
} catch { "unknown" }
Write-Host "Measuring $Measured"

try {
    # ---- the questions, as speech (synthesized once, then kept) -----------
    $QuestionsFile = if ($Questions) { $Questions } else { Join-Path $Here "questions.txt" }
    $Lines = if (Test-Path $QuestionsFile) {
        Get-Content -Encoding UTF8 $QuestionsFile | Where-Object { $_.Trim() }
    } else { $BuiltIn }
    # (MapleSyrup's own: OPENAI_API_KEY when set, else its settings' file.)
    $OpenAiKey = if ($env:OPENAI_API_KEY -like "sk-*") { $env:OPENAI_API_KEY.Trim() } else { Read-Key (Join-Path $Settings "openai-key.txt") }
    $Sha = [Security.Cryptography.SHA1]::Create()

    function Get-Speech([string]$Text) {
        $hash = -join ($Sha.ComputeHash([Text.Encoding]::UTF8.GetBytes("$TtsVoice|$Text")) | ForEach-Object { $_.ToString("x2") })
        $file = Join-Path $AudioDir "$hash.pcm"
        if (-not (Test-Path $file)) {
            $body = @{ model = "gpt-4o-mini-tts"; voice = $TtsVoice; input = $Text; response_format = "pcm" } | ConvertTo-Json -Compress
            Invoke-WebRequest -Method Post -Uri "https://api.openai.com/v1/audio/speech" -Headers @{ Authorization = "Bearer $OpenAiKey" } `
                -Body ([Text.Encoding]::UTF8.GetBytes($body)) -ContentType "application/json; charset=utf-8" -OutFile $file -UseBasicParsing | Out-Null
        }
        $speech = [IO.File]::ReadAllBytes($file)
        $end = [BenchAudio]::SpeechEnd($speech, $SpeechFloor, [int]($SampleRate * 2 * $SpeechTailMs / 1000))
        $lead = [int]($SampleRate * 2 * $LeadSilenceMs / 1000)
        $all = New-Object byte[] ($lead + $end)
        [Array]::Copy($speech, 0, $all, $lead, $end)
        return ,$all
    }

    # ---- the call, as the phone gets it ------------------------------------
    $Call = Invoke-Link "POST" "/api/live" @{ recent = @(); lang = $Lang; mic = $Mic }
    if ($Call.api -ne "ga") { throw "this account's realtime API is the old one ($($Call.api)): not measured" }
    $Socket = New-Object BenchSocket
    $Socket.Connect("wss://api.openai.com/v1/realtime?model=$($Call.model)", $Call.key)

    function Read-Events {
        $out = New-Object System.Collections.ArrayList
        $e = $null
        while ($Socket.Events.TryDequeue([ref]$e)) {
            $parsed = $e.Json | ConvertFrom-Json
            [void]$out.Add([pscustomobject]@{ At = $e.At; Type = $parsed.type; Event = $parsed })
        }
        return ,$out
    }

    # The session as created, then audio over the WebSocket as raw 24 kHz
    # PCM, and this run's overrides.
    Start-Sleep -Milliseconds 800
    $Opening = Read-Events
    $Created = $Opening | Where-Object { $_.Type -eq "session.created" } | Select-Object -First 1
    $Update = @{ type = "realtime"; audio = @{ input = @{ format = @{ type = "audio/pcm"; rate = $SampleRate } }; output = @{ format = @{ type = "audio/pcm"; rate = $SampleRate } } } }
    if ($Eagerness) { $Update.audio.input.turn_detection = @{ type = "semantic_vad"; eagerness = $Eagerness; create_response = $true; interrupt_response = $true } }
    if ($Effort) { $Update.reasoning = @{ effort = $Effort } }
    $Socket.Send((@{ type = "session.update"; session = $Update } | ConvertTo-Json -Depth 10 -Compress))
    Start-Sleep -Milliseconds 500
    $Opening += Read-Events
    $Session = ($Opening | Where-Object { $_.Type -eq "session.updated" } | Select-Object -Last 1).Event.session
    if (-not $Session) { $Session = $Created.Event.session }

    # Whether a line is in the phone's language, by its letters (a language
    # in Latin letters: Latin letters); None for a line without letters.
    $Scripts = @{ he = '[\u0590-\u05FF]'; ru = '[\u0400-\u04FF]'; uk = '[\u0400-\u04FF]'; ko = '[\uAC00-\uD7AF\u1100-\u11FF]'; ja = '[\u3040-\u30FF\u4E00-\u9FFF]'; zh = '[\u4E00-\u9FFF]'; th = '[\u0E00-\u0E7F]' }
    $LangCode = ($Lang -split '[-_]')[0].ToLower()
    $Letters = if ($Scripts.ContainsKey($LangCode)) { $Scripts[$LangCode] } else { '[A-Za-z]' }
    function Test-InLanguage([string]$Text) {
        $all = [regex]::Matches($Text, '\p{L}').Count
        if ($all -eq 0) { return $null }
        return ([regex]::Matches($Text, $Letters).Count / $all) -ge 0.6
    }

    $Turns = New-Object System.Collections.ArrayList
    $AllEvents = New-Object System.Collections.ArrayList
    $Index = 0
    foreach ($Text in $Lines) {
        $Index++
        Write-Host ("[{0}/{1}] {2}" -f $Index, $Lines.Count, $Text)
        $Pcm = Get-Speech $Text
        [void](Read-Events)
        # What the phone sends as the player starts talking: the game as read.
        try {
            $Eyes = Invoke-Link "GET" "/api/eyes?image=0" $null
            $Reading = "[The game right now, read by your vision engine — not said by the player]`n" + $Eyes.snapshot
            $Socket.Send((@{ type = "conversation.item.create"; item = @{ type = "message"; role = "user"; content = @(@{ type = "input_text"; text = $Reading }) } } | ConvertTo-Json -Depth 10 -Compress))
        } catch { }
        $DoneBefore = $Socket.Done
        $Began = [BenchClock]::Ms()
        $SpeechEnd = $Socket.Stream($Pcm, $SampleRate, $ChunkMs)
        $Socket.Silence($SampleRate, $ChunkMs, $DoneBefore, 1, $TurnMaxMs)
        $Events = Read-Events
        $Turn = [ordered]@{ question = $Text; speech_ms = [math]::Round($SpeechEnd - $Began) }
        $first = { param($events, $types) ($events | Where-Object { $types -contains $_.Type } | Select-Object -First 1) }
        $vad = & $first $Events @("input_audio_buffer.speech_stopped")
        $created = & $first $Events @("response.created")
        $sound = & $first $Events @("response.output_audio.delta", "response.audio.delta")
        $Turn.vad_ms = if ($vad) { [math]::Round($vad.At - $SpeechEnd) } else { $null }
        $Turn.response_ms = if ($created) { [math]::Round($created.At - $SpeechEnd) } else { $null }
        $Turn.first_sound_ms = if ($sound) { [math]::Round($sound.At - $SpeechEnd) } else { $null }
        $heard = $Events | Where-Object { $_.Type -eq "conversation.item.input_audio_transcription.completed" } | Select-Object -First 1
        $Turn.heard = if ($heard) { $heard.Event.transcript } else { "" }
        $said = $Events | Where-Object { $_.Type -in @("response.output_audio_transcript.done", "response.audio_transcript.done") } | ForEach-Object { $_.Event.transcript }
        $Turn.said = ($said -join " ")
        $Turn.in_language = Test-InLanguage $Turn.said
        foreach ($e in $Events) { [void]$AllEvents.Add([pscustomobject]@{ turn = $Index; at = [math]::Round($e.At - $SpeechEnd); type = $e.Type }) }

        # Tools the call asked for: run by MapleSyrup, each answer sent back
        # and the next first sound timed from there.
        $done = $Events | Where-Object { $_.Type -eq "response.done" } | Select-Object -Last 1
        $calls = @()
        if ($done) { $calls = @($done.Event.response.output | Where-Object { $_.type -eq "function_call" }) }
        $Turn.tools = @()
        $Turn.kept_quiet = $false
        foreach ($c in $calls) {
            # Not said to it: it says nothing, and, as on the phone, nothing
            # more is asked of it.
            if ($Call.quiet -and $c.name -eq $Call.quiet) {
                $Socket.Send((@{ type = "conversation.item.create"; item = @{ type = "function_call_output"; call_id = $c.call_id; output = "ok" } } | ConvertTo-Json -Depth 10 -Compress))
                $Turn.kept_quiet = $true
                continue
            }
            $ranFrom = [BenchClock]::Ms()
            $output = if ($ToolsRun -contains $c.name) {
                try { (Invoke-Link "POST" "/api/tool" @{ name = $c.name; arguments = $c.arguments }).output } catch { "That didn't work." }
            } else { "Done." }
            $ranTo = [BenchClock]::Ms()
            $doneBefore = $Socket.Done
            $Socket.Send((@{ type = "conversation.item.create"; item = @{ type = "function_call_output"; call_id = $c.call_id; output = [string]$output } } | ConvertTo-Json -Depth 10 -Compress))
            $Socket.Send('{"type":"response.create"}')
            $sentAt = [BenchClock]::Ms()
            $Socket.Silence($SampleRate, $ChunkMs, $doneBefore, 1, $TurnMaxMs)
            $after = Read-Events
            $next = & $first $after @("response.output_audio.delta", "response.audio.delta")
            $words = $after | Where-Object { $_.Type -in @("response.output_audio_transcript.done", "response.audio_transcript.done") } | ForEach-Object { $_.Event.transcript }
            $Turn.tools += [ordered]@{
                name = $c.name; arguments = $c.arguments; output = [string]$output
                ran_ms = [math]::Round($ranTo - $ranFrom)
                next_sound_ms = if ($next) { [math]::Round($next.At - $sentAt) } else { $null }
                said_after = ($words -join " ")
                from_speech_end_ms = if ($next) { [math]::Round($next.At - $SpeechEnd) } else { $null }
            }
            foreach ($e in $after) { [void]$AllEvents.Add([pscustomobject]@{ turn = $Index; at = [math]::Round($e.At - $SpeechEnd); type = $e.Type }) }
        }
        [void]$Turns.Add($Turn)
        Write-Host ("    stop heard after {0} ms; first sound {1} ms after the question ended" -f $Turn.vad_ms, $Turn.first_sound_ms)
        Start-Sleep -Milliseconds $QuietBetweenMs
    }

    $saidOf = { param($events) (($events | Where-Object { $_.Type -in @("response.output_audio_transcript.done", "response.audio_transcript.done") } | ForEach-Object { $_.Event.transcript }) -join " ") }
    $heardOf = { param($events) (($events | Where-Object { $_.Type -eq "conversation.item.input_audio_transcription.completed" } | ForEach-Object { $_.Event.transcript }) -join " | ") }

    # ---- a room with nobody talking: does it answer noise? ----------------
    Write-Host "[noise] a room with nobody talking"
    [void](Read-Events)
    $doneBefore = $Socket.Done
    $noiseFrom = [BenchClock]::Ms()
    [void]$Socket.Stream([BenchAudio]::Noise($SampleRate, $NoiseMs, $NoiseRms, 7), $SampleRate, $ChunkMs)
    $Socket.Quiet($SampleRate, $ChunkMs, $AfterNoiseMs, $false, 0)
    $events = Read-Events
    $answered = @($events | Where-Object { $_.Type -eq "response.created" }).Count
    # (A quiet tool called on the noise is answered as the phone would.)
    foreach ($d in @($events | Where-Object { $_.Type -eq "response.done" })) {
        foreach ($c in @($d.Event.response.output | Where-Object { $_.type -eq "function_call" -and $_.name -eq $Call.quiet })) {
            $Socket.Send((@{ type = "conversation.item.create"; item = @{ type = "function_call_output"; call_id = $c.call_id; output = "ok" } } | ConvertTo-Json -Depth 10 -Compress))
        }
    }
    if ($answered -gt 0) {
        # (An answer under way is let finish.)
        $Socket.Silence($SampleRate, $ChunkMs, $doneBefore, $answered, $TurnMaxMs)
        $more = Read-Events
        $events = @($events) + @($more)
    }
    $NoiseTurn = [ordered]@{
        noise_ms = $NoiseMs; noise_db = [math]::Round(20 * [math]::Log10($NoiseRms))
        heard_speech = @($events | Where-Object { $_.Type -eq "input_audio_buffer.speech_started" }).Count
        answered = $answered
        spoke = @($events | Where-Object { $_.Type -in @("response.output_audio.delta", "response.audio.delta") }).Count -gt 0
        heard = (& $heardOf $events); said = (& $saidOf $events)
    }
    foreach ($e in $events) { [void]$AllEvents.Add([pscustomobject]@{ turn = "noise"; at = [math]::Round($e.At - $noiseFrom); type = $e.Type }) }
    Write-Host ("    heard speech {0} times, answered {1} times, spoke: {2}" -f $NoiseTurn.heard_speech, $NoiseTurn.answered, $NoiseTurn.spoke)
    Start-Sleep -Milliseconds $QuietBetweenMs

    # ---- a pause mid-sentence: does it jump in? -----------------------------
    Write-Host "[pause] a pause in the middle of a sentence"
    $firstHalf = Get-Speech $PauseFirst
    $secondHalf = Get-Speech $PauseSecond
    [void](Read-Events)
    $doneBefore = $Socket.Done
    $firstEnd = $Socket.Stream($firstHalf, $SampleRate, $ChunkMs)
    $Socket.Quiet($SampleRate, $ChunkMs, $PauseMs, $false, 0)
    $secondVoice = [BenchClock]::Ms() + $LeadSilenceMs
    $secondEnd = $Socket.Stream($secondHalf, $SampleRate, $ChunkMs)
    # Until every answer begun has finished (one cut short, and the one to
    # the whole sentence).
    $events = @()
    $deadline = [BenchClock]::Ms() + $TurnMaxMs
    do {
        $Socket.Quiet($SampleRate, $ChunkMs, 300, $false, 0)
        $more = Read-Events
        $events = @($events) + @($more)
        $begun = @($events | Where-Object { $_.Type -eq "response.created" }).Count
    } while ([BenchClock]::Ms() -lt $deadline -and -not ($begun -gt 0 -and $Socket.Done -ge $doneBefore + $begun))
    $jumped = $events | Where-Object { $_.Type -eq "response.created" -and $_.At -lt $secondVoice } | Select-Object -First 1
    $pauseSound = $events | Where-Object { $_.Type -in @("response.output_audio.delta", "response.audio.delta") -and $_.At -ge $secondEnd } | Select-Object -First 1
    $PauseTurn = [ordered]@{
        pause_ms = $PauseMs + $LeadSilenceMs
        jumped_in = [bool]$jumped
        jumped_after_ms = if ($jumped) { [math]::Round($jumped.At - $firstEnd) } else { $null }
        first_sound_ms = if ($pauseSound) { [math]::Round($pauseSound.At - $secondEnd) } else { $null }
        heard = (& $heardOf $events); said = (& $saidOf @($events | Where-Object { $_.At -ge $secondEnd }))
    }
    foreach ($e in $events) { [void]$AllEvents.Add([pscustomobject]@{ turn = "pause"; at = [math]::Round($e.At - $firstEnd); type = $e.Type }) }
    Write-Host ("    jumped in during the pause: {0}; answered {1} ms after the sentence ended" -f $PauseTurn.jumped_in, $PauseTurn.first_sound_ms)
    Start-Sleep -Milliseconds $QuietBetweenMs

    # ---- talking over a long answer: how soon does it notice? --------------
    Write-Host "[over] talking over a long answer"
    $long = Get-Speech $LongQuestion
    $over = Get-Speech $OverQuestion
    [void](Read-Events)
    $doneBefore = $Socket.Done
    $longEnd = $Socket.Stream($long, $SampleRate, $ChunkMs)
    $Socket.Quiet($SampleRate, $ChunkMs, $TurnMaxMs, $true, $Socket.Sounds)
    $Socket.Quiet($SampleRate, $ChunkMs, $OverAfterMs, $false, 0)
    $overFrom = [BenchClock]::Ms()
    $overVoice = $overFrom + $LeadSilenceMs
    $overEnd = $Socket.Stream($over, $SampleRate, $ChunkMs)
    $Socket.Silence($SampleRate, $ChunkMs, $doneBefore, 2, $TurnMaxMs)
    $events = Read-Events
    $firstDone = $events | Where-Object { $_.Type -eq "response.done" } | Select-Object -First 1
    $noticed = $events | Where-Object { $_.Type -eq "input_audio_buffer.speech_started" -and $_.At -ge $overFrom } | Select-Object -First 1
    $nextSound = $events | Where-Object { $_.Type -in @("response.output_audio.delta", "response.audio.delta") -and $_.At -ge $overEnd } | Select-Object -First 1
    $longSound = $events | Where-Object { $_.Type -in @("response.output_audio.delta", "response.audio.delta") } | Select-Object -First 1
    $after = @($events | Where-Object { $_.At -ge $overEnd })
    $OverTurn = [ordered]@{
        long_first_sound_ms = if ($longSound) { [math]::Round($longSound.At - $longEnd) } else { $null }
        long_answer = if ($firstDone) { $firstDone.Event.response.status } else { $null }
        long_done_before_over = if ($firstDone) { $firstDone.At -lt $overVoice } else { $null }
        noticed_ms = if ($noticed) { [math]::Round($noticed.At - $overVoice) } else { $null }
        next_first_sound_ms = if ($nextSound) { [math]::Round($nextSound.At - $overEnd) } else { $null }
        heard = (& $heardOf $events); said_after = (& $saidOf $after)
    }
    $OverTurn.in_language = Test-InLanguage $OverTurn.said_after
    foreach ($e in $events) { [void]$AllEvents.Add([pscustomobject]@{ turn = "over"; at = [math]::Round($e.At - $overVoice); type = $e.Type }) }
    Write-Host ("    noticed {0} ms after the player began; answered {1} ms after they stopped" -f $OverTurn.noticed_ms, $OverTurn.next_first_sound_ms)
    $Socket.Close()

    # ---- the report ----------------------------------------------------------
    $Stamp = Get-Date -Format "yyyy-MM-dd HH-mm-ss"
    $Name = if ($Label) { "$Stamp $Label" } else { $Stamp }
    $Report = [ordered]@{
        when = (Get-Date -Format o); label = $Label; measured = $Measured; model = $Call.model
        session = [ordered]@{ reasoning = $Session.reasoning; turn_detection = $Session.audio.input.turn_detection; noise_reduction = $Session.audio.input.noise_reduction; transcription = $Session.audio.input.transcription; voice = $Session.audio.output.voice; speed = $Session.audio.output.speed }
        turns = $Turns; noise = $NoiseTurn; pause = $PauseTurn; talk_over = $OverTurn; events = $AllEvents; error = $Socket.Error
    }
    $Report | ConvertTo-Json -Depth 12 | Set-Content -Encoding UTF8 (Join-Path $ResultsDir "$Name.json")
    $sounds = @($Turns | Where-Object { $null -ne $_.first_sound_ms } | ForEach-Object { $_.first_sound_ms } | Sort-Object)
    $vads = @($Turns | Where-Object { $null -ne $_.vad_ms } | ForEach-Object { $_.vad_ms } | Sort-Object)
    $median = { param($xs) if ($xs.Count) { $xs[[int][math]::Floor(($xs.Count - 1) / 2)] } else { $null } }
    $Summary = @(
        "MapleSyrup live call bench — $Name",
        "measured: $Measured",
        "model $($Call.model); reasoning $($Session.reasoning.effort); turn detection $($Session.audio.input.turn_detection.type) $($Session.audio.input.turn_detection.eagerness); noise reduction $($Session.audio.input.noise_reduction.type)",
        ("first sound after the question ended: median {0} ms, min {1}, max {2} (n={3})" -f (& $median $sounds), ($sounds | Select-Object -First 1), ($sounds | Select-Object -Last 1), $sounds.Count),
        ("of which hearing the question stop: median {0} ms" -f (& $median $vads)),
        ("answers in the phone's language ({0}): {1} of {2}" -f $Lang, @($Turns | Where-Object { $_.in_language -eq $true }).Count, @($Turns | Where-Object { $null -ne $_.in_language }).Count),
        ("a room with nobody talking ({0} dB, {1} s): heard speech {2} times, answered {3} times, spoke: {4}{5}" -f $NoiseTurn.noise_db, ($NoiseTurn.noise_ms / 1000), $NoiseTurn.heard_speech, $NoiseTurn.answered, $NoiseTurn.spoke, $(if ($NoiseTurn.said) { ": " + $NoiseTurn.said } else { "" })),
        ("a pause of {0} ms mid-sentence: jumped in: {1}{2}; the whole sentence answered {3} ms after it ended: {4}" -f $PauseTurn.pause_ms, $PauseTurn.jumped_in, $(if ($PauseTurn.jumped_in) { " (" + $PauseTurn.jumped_after_ms + " ms into the pause)" } else { "" }), $PauseTurn.first_sound_ms, $PauseTurn.said),
        ("talking over a long answer ({0}): noticed {1} ms after the player began; the new question answered {2} ms after it ended: {3}" -f $OverTurn.long_answer, $OverTurn.noticed_ms, $OverTurn.next_first_sound_ms, $OverTurn.said_after),
        ""
    )
    foreach ($t in $Turns) {
        $Summary += ("{0,6} ms  {1}{2}" -f $t.first_sound_ms, $t.question, $(if ($t.kept_quiet) { "  (kept quiet: took it as not said to it)" } else { "" }))
        $Summary += ("          heard: {0}" -f $t.heard)
        $Summary += ("          said:  {0}" -f $t.said)
        foreach ($tool in $t.tools) {
            $Summary += ("          tool {0} ran {1} ms; next sound {2} ms later ({3} ms after the question): {4}" -f $tool.name, $tool.ran_ms, $tool.next_sound_ms, $tool.from_speech_end_ms, $tool.said_after)
        }
    }
    $Summary | Set-Content -Encoding UTF8 (Join-Path $ResultsDir "$Name.txt")
    $Summary | Write-Host
} finally {
    if ($Started) { Stop-Process -Id $Started.Id -Force -ErrorAction SilentlyContinue }
}
