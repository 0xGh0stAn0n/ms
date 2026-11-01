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

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File bench_live.ps1
#>
param(
    [string]$Questions = "",
    [string]$Effort = "",
    [string]$Eagerness = "",
    [string]$Label = "",
    [string]$Mic = "far",
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

    public void Close() {
        try { socket.CloseAsync(WebSocketCloseStatus.NormalClosure, "", CancellationToken.None).Wait(2000); } catch { }
    }
}
"@

[BenchTls]::TrustLocalLink()

# ---- MapleSyrup: running, or started for the run --------------------------
$LinkKey = (Get-Content -Raw (Join-Path $Settings "link-key.txt")).Trim()
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

try {
    # ---- the questions, as speech (synthesized once, then kept) -----------
    $QuestionsFile = if ($Questions) { $Questions } else { Join-Path $Here "questions.txt" }
    $Lines = if (Test-Path $QuestionsFile) {
        Get-Content -Encoding UTF8 $QuestionsFile | Where-Object { $_.Trim() }
    } else { $BuiltIn }
    $OpenAiKey = (Get-Content -Raw (Join-Path $Settings "openai-key.txt")).Trim()
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
    $Call = Invoke-Link "POST" "/api/live" @{ recent = @(); lang = "he-IL"; mic = $Mic }
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
        foreach ($e in $Events) { [void]$AllEvents.Add([pscustomobject]@{ turn = $Index; at = [math]::Round($e.At - $SpeechEnd); type = $e.Type }) }

        # Tools the call asked for: run by MapleSyrup, each answer sent back
        # and the next first sound timed from there.
        $done = $Events | Where-Object { $_.Type -eq "response.done" } | Select-Object -Last 1
        $calls = @()
        if ($done) { $calls = @($done.Event.response.output | Where-Object { $_.type -eq "function_call" }) }
        $Turn.tools = @()
        foreach ($c in $calls) {
            $ranFrom = [BenchClock]::Ms()
            $output = try { (Invoke-Link "POST" "/api/tool" @{ name = $c.name; arguments = $c.arguments }).output } catch { "That didn't work." }
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
    $Socket.Close()

    # ---- the report ----------------------------------------------------------
    $Stamp = Get-Date -Format "yyyy-MM-dd HH-mm-ss"
    $Name = if ($Label) { "$Stamp $Label" } else { $Stamp }
    $Report = [ordered]@{
        when = (Get-Date -Format o); label = $Label; model = $Call.model
        session = [ordered]@{ reasoning = $Session.reasoning; turn_detection = $Session.audio.input.turn_detection; noise_reduction = $Session.audio.input.noise_reduction; transcription = $Session.audio.input.transcription; voice = $Session.audio.output.voice; speed = $Session.audio.output.speed }
        turns = $Turns; events = $AllEvents; error = $Socket.Error
    }
    $Report | ConvertTo-Json -Depth 12 | Set-Content -Encoding UTF8 (Join-Path $ResultsDir "$Name.json")
    $sounds = @($Turns | Where-Object { $null -ne $_.first_sound_ms } | ForEach-Object { $_.first_sound_ms } | Sort-Object)
    $vads = @($Turns | Where-Object { $null -ne $_.vad_ms } | ForEach-Object { $_.vad_ms } | Sort-Object)
    $median = { param($xs) if ($xs.Count) { $xs[[int][math]::Floor(($xs.Count - 1) / 2)] } else { $null } }
    $Summary = @(
        "MapleSyrup live call bench — $Name",
        "model $($Call.model); reasoning $($Session.reasoning.effort); turn detection $($Session.audio.input.turn_detection.type) $($Session.audio.input.turn_detection.eagerness); noise reduction $($Session.audio.input.noise_reduction.type)",
        ("first sound after the question ended: median {0} ms, min {1}, max {2} (n={3})" -f (& $median $sounds), ($sounds | Select-Object -First 1), ($sounds | Select-Object -Last 1), $sounds.Count),
        ("of which hearing the question stop: median {0} ms" -f (& $median $vads)),
        ""
    )
    foreach ($t in $Turns) {
        $Summary += ("{0,6} ms  {1}" -f $t.first_sound_ms, $t.question)
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
