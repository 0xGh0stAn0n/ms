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

.PARAMETER Variant
  A variant of the call's instructions to measure against MapleSyrup's own
  (a JSON file: as given, or under bench\variants): "replace" is a list of
  {"find", "with"} applied to the instructions MapleSyrup gives the call
  (each "find" must be in them exactly once), "prepend" and "append" add
  text; "tools" adds tools to the call's own, and "quiet" names the one the
  call calls instead of speaking (answered as the phone answers it). Nothing
  in MapleSyrup changes: the bench's call alone is told.

.PARAMETER TtsStyle
  How the questions are spoken (the voice's instructions).

.PARAMETER UseRunning
  Measure whatever MapleSyrup is running. By default the bench measures the
  one beside it (the build under test): another MapleSyrup running (the
  installed one) is closed for the run and started again afterwards as it
  was, with its own command line.

.NOTES
  Besides the questions it plays the player talking to someone else in the
  room (does it keep out of it?), speech nobody could make out (one of the
  questions backwards, kept as audio\unclear.pcm: does it say it didn't
  catch it?), a room with nobody talking
  (does the call answer noise?), a pause mid-sentence (does it jump in?) and
  talks over a long answer (how soon does it notice?). Each starts once the
  call is quiet, and an answer is the one to what it heard last: one begun
  at a pause mid-question and cut short by the rest of it is counted, not
  timed.
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
    [switch]$UseRunning,
    [int]$Port = 8443,
    [string]$TtsVoice = "ash",
    [string]$Variant = "",
    [string]$TtsStyle = "A native speaker of the text's language, speaking it naturally and clearly, casually, like a gamer on voice chat with a friend."
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
$VariantsDir = Join-Path $Here "variants"
# Speech nobody could make out, played as a question: the call should say
# it didn't catch it. (Made once, as one of the questions spoken backwards,
# unless there is one already.)
$UnclearFile = Join-Path $AudioDir "unclear.pcm"
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
# Before each question and scenario: every answer begun finished, and none
# begun for this long (what came before is over).
$SettleMs = 800
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
    "אני חושב לעבור למפה אחרת כי פה יש יותר מדי שחקנים, יש לך רעיון לאן כדאי לי ללכת עם הרמה שלי?",
    "ספר לי בדיחה קצרה",
    "מה דעתך, כדאי לי לקחת הפסקה לאכול משהו?"
)
# The player talking to someone else in the room, not to MapleSyrup.
$Aside = "כן אמא, אני כבר בא, עוד חמש דקות"
# A question with a long answer, and what is said over it.
$LongQuestion = "תספר לי בפירוט את כל הסיפור של מייפל איילנד מההתחלה"
$OverQuestion = "רגע רגע, עצור. כמה MP יש לי?"
# A sentence with a pause in its middle, as a player thinking aloud: does
# it jump in during the pause?
$PauseFirst = "אני חושב שאולי כדאי לי"
$PauseSecond = "לעבור למפה אחרת, מה אתה אומר?"
$PauseMs = 900

# ---- a WebSocket with timestamps, and the localhost certificate ---------
# (A single-quoted here-string: PowerShell expands nothing in the C#.)
Add-Type -TypeDefinition @'
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
    // A room's noise, ms long at rms of full scale: white noise smoothed
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

    // The same speech backwards (16-bit PCM): speech-like, and nobody can
    // make it out.
    public static byte[] Backwards(byte[] pcm) {
        var back = new byte[pcm.Length - pcm.Length % 2];
        for (int i = 0, j = back.Length - 2; j >= 0; i += 2, j -= 2) {
            back[i] = pcm[j];
            back[i + 1] = pcm[j + 1];
        }
        return back;
    }

    // Where the speech in 16-bit PCM ends: tailBytes after the last sample
    // louder than floor.
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
    public volatile int Created;
    public volatile int Done;
    public volatile int Sounds;
    public volatile string Error;
    // When the call last heard the player stop, and last began an answer.
    double lastStopped = -1, lastCreated = -1;
    public double LastStopped { get { return Volatile.Read(ref lastStopped); } }
    public double LastCreated { get { return Volatile.Read(ref lastCreated); } }

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
                double at = BenchClock.Ms();
                Events.Enqueue(new BenchEvent { At = at, Json = text });
                if (text.Contains("\"type\":\"input_audio_buffer.speech_stopped\"")) Volatile.Write(ref lastStopped, at);
                if (text.Contains("\"type\":\"response.created\"")) { Volatile.Write(ref lastCreated, at); Created++; }
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

    // PCM16 mono at rate, in real time, chunkMs a chunk. Returns when
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

    // Room silence in real time until doneBefore responses more have
    // finished, or maxMs passed.
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

    // Whether the call answered what it heard after since: it heard the
    // player stop after then, began an answer after that, and every answer
    // begun has finished (one begun at a pause mid-question and cut short
    // by the rest of it is not the answer).
    public bool Answered(double since) {
        double stopped = LastStopped;
        return stopped >= since && LastCreated >= stopped && Created == Done;
    }

    // Room silence in real time until the call answered what it heard after
    // since, or maxMs passed.
    public void Listen(int rate, int chunkMs, double since, int maxMs) {
        var silence = new byte[rate * 2 * chunkMs / 1000];
        double start = BenchClock.Ms();
        int n = 0;
        while (!Answered(since) && BenchClock.Ms() - start < maxMs && Error == null) {
            double due = start + n * chunkMs;
            while (BenchClock.Ms() < due) Thread.Sleep(1);
            Chunk(silence, 0, silence.Length);
            n++;
        }
    }

    // Room silence in real time until every answer begun has finished and
    // none began for quietMs, or maxMs passed: what came before is over.
    public void Settle(int rate, int chunkMs, int quietMs, int maxMs) {
        var silence = new byte[rate * 2 * chunkMs / 1000];
        double start = BenchClock.Ms();
        double still = start;
        int n = 0;
        while (BenchClock.Ms() - start < maxMs && Error == null) {
            if (Created != Done) still = BenchClock.Ms();
            if (BenchClock.Ms() - still >= quietMs) break;
            double due = start + n * chunkMs;
            while (BenchClock.Ms() < due) Thread.Sleep(1);
            Chunk(silence, 0, silence.Length);
            n++;
        }
    }

    // Room silence in real time for ms, or until a sound more than
    // soundsBefore came (untilSound).
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
'@

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

# The build under test is the one beside this bench: another MapleSyrup
# running is closed for the run (asked first, then made to), and started
# again afterwards as it was.
$Restore = @()
$Started = $null
# (Whatever happens from here, what was running is started again.)
try {
if (-not $UseRunning) {
    $others = @(Get-CimInstance Win32_Process -Filter "Name = 'MapleSyrup.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.ExecutablePath -and $_.ExecutablePath -ne $Exe })
    foreach ($o in $others) {
        $Restore += [pscustomobject]@{ Path = $o.ExecutablePath; CommandLine = $o.CommandLine }
        Write-Host "Closing $($o.ExecutablePath) for the run (it starts again afterwards)"
        $p = Get-Process -Id $o.ProcessId -ErrorAction SilentlyContinue
        if ($p) {
            [void]$p.CloseMainWindow()
            if (-not $p.WaitForExit(5000)) { Stop-Process -Id $o.ProcessId -Force -ErrorAction SilentlyContinue }
        }
    }
    if ($others.Count -gt 0) {
        $deadline = (Get-Date).AddSeconds(15)
        while ((Test-Link) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 300 }
    }
}

if (-not (Test-Link)) {
    Write-Host "Starting MapleSyrup for the run..."
    $Started = Start-Process -FilePath $Exe -ArgumentList "--no-overlay", "--no-voice", "--plain", "--no-update", "--latency" -WorkingDirectory $AppDir -WindowStyle Minimized -PassThru
    $deadline = (Get-Date).AddSeconds(60)
    while (-not (Test-Link)) {
        if ((Get-Date) -gt $deadline) { throw "MapleSyrup's phone link did not come up on port $Port" }
        Start-Sleep -Milliseconds 500
    }
}

# Which MapleSyrup is measured: its program and when it was built.
$Measured = try {
    $proc = Get-Process -Name "MapleSyrup" -ErrorAction Stop | Select-Object -First 1
    "{0} ({1})" -f $proc.Path, (Get-Item $proc.Path).LastWriteTime.ToString("yyyy-MM-dd HH:mm")
} catch { "unknown" }
Write-Host "Measuring $Measured"

    # ---- the questions, as speech (synthesized once, then kept) -----------
    $QuestionsFile = if ($Questions) { $Questions } else { Join-Path $Here "questions.txt" }
    $Lines = if (Test-Path $QuestionsFile) {
        Get-Content -Encoding UTF8 $QuestionsFile | Where-Object { $_.Trim() }
    } else { $BuiltIn }
    # (MapleSyrup's own: OPENAI_API_KEY when set, else its settings' file.)
    $OpenAiKey = if ($env:OPENAI_API_KEY -like "sk-*") { $env:OPENAI_API_KEY.Trim() } else { Read-Key (Join-Path $Settings "openai-key.txt") }
    $Sha = [Security.Cryptography.SHA1]::Create()

    # Speech as a question goes: room silence first, and after its last
    # sound only a little (the voice pads its end with silence, which is not
    # the player still talking).
    function Format-Speech([byte[]]$Speech) {
        $end = [BenchAudio]::SpeechEnd($Speech, $SpeechFloor, [int]($SampleRate * 2 * $SpeechTailMs / 1000))
        $lead = [int]($SampleRate * 2 * $LeadSilenceMs / 1000)
        $all = New-Object byte[] ($lead + $end)
        [Array]::Copy($Speech, 0, $all, $lead, $end)
        return ,$all
    }

    # The file of Text spoken (synthesized the first time).
    function Get-SpeechFile([string]$Text) {
        $hash = -join ($Sha.ComputeHash([Text.Encoding]::UTF8.GetBytes("$TtsVoice|$TtsStyle|$Text")) | ForEach-Object { $_.ToString("x2") })
        $file = Join-Path $AudioDir "$hash.pcm"
        if (-not (Test-Path $file)) {
            $body = @{ model = "gpt-4o-mini-tts"; voice = $TtsVoice; input = $Text; instructions = $TtsStyle; response_format = "pcm" } | ConvertTo-Json -Compress
            Invoke-WebRequest -Method Post -Uri "https://api.openai.com/v1/audio/speech" -Headers @{ Authorization = "Bearer $OpenAiKey" } `
                -Body ([Text.Encoding]::UTF8.GetBytes($body)) -ContentType "application/json; charset=utf-8" -OutFile $file -UseBasicParsing | Out-Null
        }
        return $file
    }

    function Get-Speech([string]$Text) {
        $pcm = Format-Speech ([IO.File]::ReadAllBytes((Get-SpeechFile $Text)))
        return ,$pcm
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
    # A variant of the call's instructions: MapleSyrup's own, as it gives
    # them to the phone, changed as the variant says.
    $Instructions = $null
    if ($Variant) {
        $variantFile = @($Variant, (Join-Path $VariantsDir $Variant)) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf -ErrorAction SilentlyContinue } | Select-Object -First 1
        if (-not $variantFile) { throw "variant not found: $Variant" }
        $variantSpec = Get-Content -Raw -Encoding UTF8 -LiteralPath $variantFile | ConvertFrom-Json
        $Instructions = (Invoke-Link "GET" "/api/instructions?lang=$Lang" $null).instructions
        foreach ($change in @($variantSpec.replace)) {
            if (-not $change) { continue }
            $at = $Instructions.IndexOf($change.find, [StringComparison]::Ordinal)
            if ($at -lt 0 -or $Instructions.IndexOf($change.find, $at + 1, [StringComparison]::Ordinal) -ge 0) {
                throw "variant ${Variant}: not once in the call's instructions: $($change.find)"
            }
            $Instructions = $Instructions.Substring(0, $at) + $change.with + $Instructions.Substring($at + $change.find.Length)
        }
        if ($variantSpec.prepend) { $Instructions = $variantSpec.prepend + "`n`n" + $Instructions }
        if ($variantSpec.append) { $Instructions = $Instructions + "`n`n" + $variantSpec.append }
        if ($variantSpec.tools) {
            $own = @($Created.Event.session.tools | Where-Object { $_ })
            if ($own.Count -eq 0) { throw "variant ${Variant}: the call's own tools are not known, so none are added" }
            $Update.tools = @($own) + @($variantSpec.tools)
            Write-Host ("Variant {0}: {1} tools, {2} of them its own" -f $Variant, $Update.tools.Count, $own.Count)
        }
        if ($variantSpec.quiet) { $Call | Add-Member -NotePropertyName quiet -NotePropertyValue $variantSpec.quiet -Force }
        $Update.instructions = $Instructions
        Write-Host ("Variant {0}: {1} characters of instructions" -f $Variant, $Instructions.Length)
    }
    # (Deep enough for the tools' schemas: deeper ones would go as text.)
    $Socket.Send((@{ type = "session.update"; session = $Update } | ConvertTo-Json -Depth 50 -Compress))
    $deadline = [BenchClock]::Ms() + 5000
    do {
        Start-Sleep -Milliseconds 250
        $Opening += Read-Events
        $updated = @($Opening | Where-Object { $_.Type -eq "session.updated" -or $_.Type -eq "error" })
    } while ($updated.Count -eq 0 -and [BenchClock]::Ms() -lt $deadline)
    $UpdateErrors = @($Opening | Where-Object { $_.Type -eq "error" } | ForEach-Object { $_.Event.error.message })
    foreach ($why in $UpdateErrors) { Write-Host "The call refused this run's settings: $why" }
    $Session = ($Opening | Where-Object { $_.Type -eq "session.updated" } | Select-Object -Last 1).Event.session
    if (-not $Session) { $Session = $Created.Event.session }
    # A variant the call did not take is not measured.
    if ($Instructions) {
        foreach ($change in @($variantSpec.replace)) {
            $mark = ([string]$change.with).Split("`n")[0]
            if ($mark -and -not ([string]$Session.instructions).Contains($mark)) { throw "variant ${Variant}: the call did not take its instructions ($($UpdateErrors -join '; '))" }
        }
    }
    if ($variantSpec.quiet -and -not @($Session.tools | Where-Object { $_.name -eq $variantSpec.quiet })) { throw "variant ${Variant}: the call did not take its $($variantSpec.quiet) tool" }

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

    $SoundTypes = @("response.output_audio.delta", "response.audio.delta")
    $SaidTypes = @("response.output_audio_transcript.done", "response.audio_transcript.done")
    # The answer to what the call heard last: the last stop it heard, the
    # first answer begun after it, and that answer's first sound and words
    # (an answer begun at a pause mid-question and cut short by the rest of
    # it is counted, not timed).
    function Get-Answer($Events) {
        $stop = $Events | Where-Object { $_.Type -eq "input_audio_buffer.speech_stopped" } | Select-Object -Last 1
        $created = if ($stop) { $Events | Where-Object { $_.Type -eq "response.created" -and $_.At -ge $stop.At } | Select-Object -First 1 } else { $null }
        $id = if ($created) { $created.Event.response.id } else { $null }
        $sound = if ($id) { $Events | Where-Object { $SoundTypes -contains $_.Type -and $_.Event.response_id -eq $id } | Select-Object -First 1 } else { $null }
        $words = if ($id) { @($Events | Where-Object { $SaidTypes -contains $_.Type -and $_.Event.response_id -eq $id } | ForEach-Object { $_.Event.transcript }) } else { @() }
        $early = if ($stop) { @($Events | Where-Object { $_.Type -eq "response.created" -and $_.At -lt $stop.At }).Count } else { 0 }
        return [pscustomobject]@{ Stop = $stop; Created = $created; Sound = $sound; Said = ($words -join " "); Early = $early }
    }

    $Turns = New-Object System.Collections.ArrayList
    $AllEvents = New-Object System.Collections.ArrayList
    $Specs = @($Lines | ForEach-Object { [pscustomobject]@{ Text = $_; Unclear = $false; Aside = $false } })
    $Specs += [pscustomobject]@{ Text = $Aside; Unclear = $false; Aside = $true }
    if (-not (Test-Path $UnclearFile)) {
        [IO.File]::WriteAllBytes($UnclearFile, [BenchAudio]::Backwards([IO.File]::ReadAllBytes((Get-SpeechFile $BuiltIn[2]))))
    }
    $Specs += [pscustomobject]@{ Text = "(speech nobody could make out)"; Unclear = $true; Aside = $false }
    $Index = 0
    foreach ($Spec in $Specs) {
        $Index++
        $Text = $Spec.Text
        Write-Host ("[{0}/{1}] {2}" -f $Index, $Specs.Count, $Text)
        $Pcm = if ($Spec.Unclear) { Format-Speech ([IO.File]::ReadAllBytes($UnclearFile)) } else { Get-Speech $Text }
        $Socket.Settle($SampleRate, $ChunkMs, $SettleMs, $TurnMaxMs)
        [void](Read-Events)
        # What the phone sends as the player starts talking: the game as read.
        try {
            $Eyes = Invoke-Link "GET" "/api/eyes?image=0" $null
            $Reading = "[The game right now, read by your vision engine — not said by the player]`n" + $Eyes.snapshot
            $Socket.Send((@{ type = "conversation.item.create"; item = @{ type = "message"; role = "user"; content = @(@{ type = "input_text"; text = $Reading }) } } | ConvertTo-Json -Depth 10 -Compress))
        } catch { }
        $Began = [BenchClock]::Ms()
        $SpeechEnd = $Socket.Stream($Pcm, $SampleRate, $ChunkMs)
        $Socket.Listen($SampleRate, $ChunkMs, $SpeechEnd, $TurnMaxMs)
        $Events = Read-Events
        $Turn = [ordered]@{ question = $Text; unclear = $Spec.Unclear; aside = $Spec.Aside; speech_ms = [math]::Round($SpeechEnd - $Began) }
        $first = { param($events, $types) ($events | Where-Object { $types -contains $_.Type } | Select-Object -First 1) }
        $answer = Get-Answer $Events
        $Turn.vad_ms = if ($answer.Stop) { [math]::Round($answer.Stop.At - $SpeechEnd) } else { $null }
        $Turn.response_ms = if ($answer.Created) { [math]::Round($answer.Created.At - $SpeechEnd) } else { $null }
        $Turn.first_sound_ms = if ($answer.Sound) { [math]::Round($answer.Sound.At - $SpeechEnd) } else { $null }
        $Turn.cut_in = $answer.Early
        $Turn.heard = (@($Events | Where-Object { $_.Type -eq "conversation.item.input_audio_transcription.completed" } | ForEach-Object { $_.Event.transcript }) -join " | ")
        $Turn.said = $answer.Said
        $Turn.in_language = Test-InLanguage $Turn.said
        foreach ($e in $Events) { [void]$AllEvents.Add([pscustomobject]@{ turn = $Index; at = [math]::Round($e.At - $SpeechEnd); type = $e.Type }) }

        # Tools the call asked for: run by MapleSyrup, each answer sent back
        # and the next first sound timed from there.
        $answerId = if ($answer.Created) { $answer.Created.Event.response.id } else { $null }
        $done = $Events | Where-Object { $_.Type -eq "response.done" -and $_.Event.response.id -eq $answerId } | Select-Object -Last 1
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
            $next = & $first $after $SoundTypes
            $words = $after | Where-Object { $SaidTypes -contains $_.Type } | ForEach-Object { $_.Event.transcript }
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
        Write-Host ("    stop heard after {0} ms; first sound {1} ms after the question ended{2}" -f $Turn.vad_ms, $Turn.first_sound_ms, $(if ($Turn.cut_in) { " (began $($Turn.cut_in) answer(s) mid-question)" } else { "" }))
        Start-Sleep -Milliseconds $QuietBetweenMs
    }

    $saidOf = { param($events) (($events | Where-Object { $SaidTypes -contains $_.Type } | ForEach-Object { $_.Event.transcript }) -join " ") }
    $heardOf = { param($events) (($events | Where-Object { $_.Type -eq "conversation.item.input_audio_transcription.completed" } | ForEach-Object { $_.Event.transcript }) -join " | ") }

    # ---- a room with nobody talking: does it answer noise? ----------------
    Write-Host "[noise] a room with nobody talking"
    $Socket.Settle($SampleRate, $ChunkMs, $SettleMs, $TurnMaxMs)
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
        spoke = @($events | Where-Object { $SoundTypes -contains $_.Type }).Count -gt 0
        heard = (& $heardOf $events); said = (& $saidOf $events)
    }
    foreach ($e in $events) { [void]$AllEvents.Add([pscustomobject]@{ turn = "noise"; at = [math]::Round($e.At - $noiseFrom); type = $e.Type }) }
    Write-Host ("    heard speech {0} times, answered {1} times, spoke: {2}" -f $NoiseTurn.heard_speech, $NoiseTurn.answered, $NoiseTurn.spoke)
    Start-Sleep -Milliseconds $QuietBetweenMs

    # ---- a pause mid-sentence: does it jump in? -----------------------------
    Write-Host "[pause] a pause in the middle of a sentence"
    $firstHalf = Get-Speech $PauseFirst
    $secondHalf = Get-Speech $PauseSecond
    $Socket.Settle($SampleRate, $ChunkMs, $SettleMs, $TurnMaxMs)
    [void](Read-Events)
    $firstEnd = $Socket.Stream($firstHalf, $SampleRate, $ChunkMs)
    $Socket.Quiet($SampleRate, $ChunkMs, $PauseMs, $false, 0)
    $secondVoice = [BenchClock]::Ms() + $LeadSilenceMs
    $secondEnd = $Socket.Stream($secondHalf, $SampleRate, $ChunkMs)
    # Until the answer to the whole sentence (any begun in the pause cut
    # short by the rest of it).
    $Socket.Listen($SampleRate, $ChunkMs, $secondEnd, $TurnMaxMs)
    $events = Read-Events
    $jumped = $events | Where-Object { $_.Type -eq "response.created" -and $_.At -lt $secondVoice } | Select-Object -First 1
    $pauseAnswer = Get-Answer $events
    $PauseTurn = [ordered]@{
        pause_ms = $PauseMs + $LeadSilenceMs
        jumped_in = [bool]$jumped
        jumped_after_ms = if ($jumped) { [math]::Round($jumped.At - $firstEnd) } else { $null }
        first_sound_ms = if ($pauseAnswer.Sound) { [math]::Round($pauseAnswer.Sound.At - $secondEnd) } else { $null }
        heard = (& $heardOf $events); said = $pauseAnswer.Said
    }
    foreach ($e in $events) { [void]$AllEvents.Add([pscustomobject]@{ turn = "pause"; at = [math]::Round($e.At - $firstEnd); type = $e.Type }) }
    Write-Host ("    jumped in during the pause: {0}; answered {1} ms after the sentence ended" -f $PauseTurn.jumped_in, $PauseTurn.first_sound_ms)
    Start-Sleep -Milliseconds $QuietBetweenMs

    # ---- talking over a long answer: how soon does it notice? --------------
    Write-Host "[over] talking over a long answer"
    $long = Get-Speech $LongQuestion
    $over = Get-Speech $OverQuestion
    $Socket.Settle($SampleRate, $ChunkMs, $SettleMs, $TurnMaxMs)
    [void](Read-Events)
    $longEnd = $Socket.Stream($long, $SampleRate, $ChunkMs)
    $Socket.Quiet($SampleRate, $ChunkMs, $TurnMaxMs, $true, $Socket.Sounds)
    $Socket.Quiet($SampleRate, $ChunkMs, $OverAfterMs, $false, 0)
    $overFrom = [BenchClock]::Ms()
    $overVoice = $overFrom + $LeadSilenceMs
    $overEnd = $Socket.Stream($over, $SampleRate, $ChunkMs)
    $Socket.Listen($SampleRate, $ChunkMs, $overEnd, $TurnMaxMs)
    $events = Read-Events
    # The long answer: the one speaking when the player began to talk over it.
    $longSound = $events | Where-Object { $SoundTypes -contains $_.Type } | Select-Object -First 1
    $longId = if ($longSound) { $longSound.Event.response_id } else { $null }
    $firstDone = $events | Where-Object { $_.Type -eq "response.done" -and $_.Event.response.id -eq $longId } | Select-Object -First 1
    $noticed = $events | Where-Object { $_.Type -eq "input_audio_buffer.speech_started" -and $_.At -ge $overFrom } | Select-Object -First 1
    $overAnswer = Get-Answer @($events | Where-Object { $_.At -ge $overFrom })
    $OverTurn = [ordered]@{
        long_first_sound_ms = if ($longSound) { [math]::Round($longSound.At - $longEnd) } else { $null }
        long_answer = if ($firstDone) { $firstDone.Event.response.status } else { $null }
        long_done_before_over = if ($firstDone) { $firstDone.At -lt $overVoice } else { $null }
        noticed_ms = if ($noticed) { [math]::Round($noticed.At - $overVoice) } else { $null }
        next_first_sound_ms = if ($overAnswer.Sound) { [math]::Round($overAnswer.Sound.At - $overEnd) } else { $null }
        heard = (& $heardOf $events); said_after = $overAnswer.Said
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
        variant = $Variant; instructions = $Instructions; update_errors = $UpdateErrors
        session = [ordered]@{ reasoning = $Session.reasoning; turn_detection = $Session.audio.input.turn_detection; noise_reduction = $Session.audio.input.noise_reduction; transcription = $Session.audio.input.transcription; voice = $Session.audio.output.voice; speed = $Session.audio.output.speed }
        turns = $Turns; noise = $NoiseTurn; pause = $PauseTurn; talk_over = $OverTurn; events = $AllEvents; error = $Socket.Error
    }
    $Report | ConvertTo-Json -Depth 12 | Set-Content -Encoding UTF8 (Join-Path $ResultsDir "$Name.json")
    # (The questions: not the aside, not the speech nobody could make out.)
    $Asked = @($Turns | Where-Object { -not $_.unclear -and -not $_.aside })
    $sounds = @($Asked | Where-Object { $null -ne $_.first_sound_ms } | ForEach-Object { $_.first_sound_ms } | Sort-Object)
    $vads = @($Asked | Where-Object { $null -ne $_.vad_ms } | ForEach-Object { $_.vad_ms } | Sort-Object)
    $asideTurn = $Turns | Where-Object { $_.aside } | Select-Object -First 1
    $unclearTurn = $Turns | Where-Object { $_.unclear } | Select-Object -First 1
    $median = { param($xs) if ($xs.Count) { $xs[[int][math]::Floor(($xs.Count - 1) / 2)] } else { $null } }
    $Summary = @(
        "MapleSyrup live call bench — $Name",
        "measured: $Measured",
        "model $($Call.model); reasoning $($Session.reasoning.effort); turn detection $($Session.audio.input.turn_detection.type) $($Session.audio.input.turn_detection.eagerness); noise reduction $($Session.audio.input.noise_reduction.type)",
        $(if ($Variant) { "instructions: MapleSyrup's, as variant $Variant has them ($($Instructions.Length) characters)" } else { "instructions: MapleSyrup's own" }),
        ("first sound after the question ended: median {0} ms, min {1}, max {2} (n={3})" -f (& $median $sounds), ($sounds | Select-Object -First 1), ($sounds | Select-Object -Last 1), $sounds.Count),
        ("of which hearing the question stop: median {0} ms; answers begun mid-question and cut short: {1}" -f (& $median $vads), (@($Turns | ForEach-Object { $_.cut_in }) | Measure-Object -Sum).Sum),
        ("answers in the phone's language ({0}): {1} of {2}" -f $Lang, @($Turns | Where-Object { $_.in_language -eq $true }).Count, @($Turns | Where-Object { $null -ne $_.in_language }).Count),
        $(if ($asideTurn) { "the player talking to someone else: " + $(if ($asideTurn.kept_quiet) { "kept quiet" } elseif ($asideTurn.said) { "said: " + $asideTurn.said } else { "said nothing" }) } else { "" }),
        $(if ($unclearTurn) { "speech nobody could make out: " + $(if ($unclearTurn.kept_quiet) { "kept quiet" } elseif ($unclearTurn.said) { "said: " + $unclearTurn.said } else { "said nothing" }) } else { "" }),
        ("a room with nobody talking ({0} dB, {1} s): heard speech {2} times, answered {3} times, spoke: {4}{5}" -f $NoiseTurn.noise_db, ($NoiseTurn.noise_ms / 1000), $NoiseTurn.heard_speech, $NoiseTurn.answered, $NoiseTurn.spoke, $(if ($NoiseTurn.said) { ": " + $NoiseTurn.said } else { "" })),
        ("a pause of {0} ms mid-sentence: jumped in: {1}{2}; the whole sentence answered {3} ms after it ended: {4}" -f $PauseTurn.pause_ms, $PauseTurn.jumped_in, $(if ($PauseTurn.jumped_in) { " (" + $PauseTurn.jumped_after_ms + " ms into the pause)" } else { "" }), $PauseTurn.first_sound_ms, $PauseTurn.said),
        ("talking over a long answer ({0}): noticed {1} ms after the player began; the new question answered {2} ms after it ended: {3}" -f $OverTurn.long_answer, $OverTurn.noticed_ms, $OverTurn.next_first_sound_ms, $OverTurn.said_after),
        ""
    )
    foreach ($t in $Turns) {
        $Summary += ("{0,6} ms  {1}{2}{3}" -f $t.first_sound_ms, $t.question, $(if ($t.kept_quiet) { "  (kept quiet: took it as not said to it)" } else { "" }), $(if ($t.cut_in) { "  (began $($t.cut_in) answer(s) mid-question)" } else { "" }))
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
    # What was running before, started again with its own arguments.
    foreach ($r in $Restore) {
        $rest = ""
        if ($r.CommandLine) {
            $line = $r.CommandLine.Trim()
            $rest = if ($line.StartsWith([string][char]34)) { $line.Substring($line.IndexOf([char]34, 1) + 1).Trim() } else { @($line -split '\s+', 2)[1] }
        }
        Start-Sleep -Milliseconds 800
        $dir = Split-Path -Parent $r.Path
        if ($rest) { Start-Process -FilePath $r.Path -ArgumentList $rest -WorkingDirectory $dir } else { Start-Process -FilePath $r.Path -WorkingDirectory $dir }
        Write-Host "Started again: $($r.Path) $rest"
    }
}
