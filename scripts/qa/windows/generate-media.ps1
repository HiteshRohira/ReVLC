# Makes the Windows QA fixture set; existing files are kept, so re-running
# only adds what's missing. See docs/audit-playbook.md.
#
#   powershell -ExecutionPolicy Bypass -File scripts\qa\windows\generate-media.ps1
#
# Fixtures go to $env:QA_MEDIA (default %USERPROFILE%\revlc-qa\media).
# ffmpeg: $env:QA_FFMPEG, else ffmpeg on PATH, else the essentials build
# under %USERPROFILE%\revlc-qa\tools or
# %USERPROFILE%\revlc-qa\win-*\tools.
$ErrorActionPreference = 'Stop'
$media = if ($env:QA_MEDIA) { $env:QA_MEDIA } else { Join-Path $env:USERPROFILE 'revlc-qa\media' }
$ff = if ($env:QA_FFMPEG) { $env:QA_FFMPEG }
      elseif (Get-Command ffmpeg.exe -ErrorAction SilentlyContinue) { (Get-Command ffmpeg.exe).Source }
      else { Get-ChildItem "$env:USERPROFILE\revlc-qa\tools", "$env:USERPROFILE\revlc-qa\win-*\tools" -Filter ffmpeg.exe -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName }
if (-not $ff) { throw 'ffmpeg not found: set $env:QA_FFMPEG' }

$dirs = 'HLS TS', 'HLS fMP4', 'Paths with spaces', 'Series\Test Show\Season 01', 'Junction Loop'
New-Item -ItemType Directory -Force -Path $media, ($dirs | ForEach-Object { Join-Path $media $_ }) | Out-Null

# Runs ffmpeg in the output's folder: fMP4 HLS writes init.mp4 relative to
# the working directory, which once landed in the repo.
function FF([string]$out, [string[]]$ffArgs) {
  $target = Join-Path $media $out
  if (Test-Path -LiteralPath $target) { return }
  Push-Location (Split-Path $target)
  try {
    & $ff -hide_banner -loglevel error -y @ffArgs $target
    if ($LASTEXITCODE -ne 0) { throw "ffmpeg failed for $out" }
  } finally { Pop-Location }
}

$src = Join-Path $media 'H264 AAC.mp4'
FF 'H264 AAC.mp4' @('-f', 'lavfi', '-i', 'testsrc2=size=640x360:rate=24', '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000', '-t', '180', '-c:v', 'libx264', '-preset', 'ultrafast', '-crf', '26', '-pix_fmt', 'yuv420p', '-c:a', 'aac', '-metadata', 'title=QA H264 AAC 3 minutes')
FF 'Long Resume.mp4' @('-f', 'lavfi', '-i', 'testsrc2=size=640x360:rate=15', '-t', '600', '-c:v', 'libx264', '-preset', 'ultrafast', '-crf', '28', '-an', '-metadata', 'title=QA Long Resume 10 minutes')
FF 'HEVC AAC.mkv' @('-i', $src, '-t', '40', '-c:v', 'libx265', '-preset', 'ultrafast', '-x265-params', 'log-level=error:pools=2', '-c:a', 'aac')
FF 'VP9 Opus.webm' @('-i', $src, '-t', '30', '-c:v', 'libvpx-vp9', '-deadline', 'realtime', '-cpu-used', '8', '-c:a', 'libopus')
FF 'MPEG4 MP3.avi' @('-i', $src, '-t', '30', '-c:v', 'mpeg4', '-q:v', '6', '-c:a', 'libmp3lame')
FF 'Audio WAV.wav' @('-f', 'lavfi', '-i', 'sine=frequency=220:sample_rate=44100', '-t', '60')
FF 'Audio FLAC.flac' @('-i', (Join-Path $media 'Audio WAV.wav'))
FF 'Audio MP3.mp3' @('-i', (Join-Path $media 'Audio WAV.wav'))
# Red/blue pair for replaced-file and stale-thumbnail checks.
FF 'Red 60.mp4' @('-f', 'lavfi', '-i', 'color=c=red:size=640x360:rate=24', '-t', '60', '-c:v', 'libx264', '-preset', 'ultrafast', '-metadata', 'title=QA RED original 60s')
FF 'Blue 90.mp4' @('-f', 'lavfi', '-i', 'color=c=blue:size=640x360:rate=24', '-t', '90', '-c:v', 'libx264', '-preset', 'ultrafast', '-metadata', 'title=QA BLUE replacement 90s')
FF 'HLS TS\stream.m3u8' @('-i', $src, '-t', '30', '-c', 'copy', '-hls_time', '6', '-hls_playlist_type', 'vod', '-hls_segment_filename', 'segment%03d.ts')
FF 'HLS fMP4\stream.m3u8' @('-i', $src, '-t', '30', '-c', 'copy', '-hls_time', '6', '-hls_playlist_type', 'vod', '-hls_segment_type', 'fmp4', '-hls_fmp4_init_filename', 'init.mp4', '-hls_segment_filename', 'segment%03d.m4s')
foreach ($d in 'HLS TS', 'HLS fMP4') {
  "#EXTM3U`n#EXT-X-STREAM-INF:BANDWIDTH=2000000,RESOLUTION=640x360`nstream.m3u8" | Set-Content (Join-Path $media "$d\master.m3u8") -Encoding ascii
}

@"
1
00:00:00,000 --> 00:01:00,000
QA external subtitle attached to correct video

2
00:01:00,000 --> 00:02:59,000
Second minute: subtitle seeking test
"@ | Set-Content (Join-Path $media 'External unrelated captions.srt') -Encoding utf8

$copies = @{
  'Paths with spaces\Unicode café हिन्दी 日本語 🎬.mp4'                    = 'H264 AAC.mp4'
  'Series\Test Show\Season 01\Test.Show.S01E01.Episode.One.mp4' = 'H264 AAC.mp4'
  'Series\Test Show\Season 01\Test.Show.S01E02.Episode.Two.mkv' = 'HEVC AAC.mkv'
  'Junction Loop\single.mp4'                                    = 'H264 AAC.mp4'
}
foreach ($to in $copies.Keys) {
  $dest = Join-Path $media $to
  if (-not (Test-Path -LiteralPath $dest)) { Copy-Item -LiteralPath (Join-Path $media $copies[$to]) -Destination $dest }
}
[IO.File]::WriteAllBytes((Join-Path $media 'Zero byte.mp4'), [byte[]]@())
[IO.File]::WriteAllBytes((Join-Path $media 'Corrupt.mp4'), [Text.Encoding]::UTF8.GetBytes('This is deliberately not a video container'))
[IO.File]::WriteAllBytes((Join-Path $media 'Unsupported.xyz'), [Text.Encoding]::UTF8.GetBytes('Unsupported QA file'))
$loop = Join-Path $media 'Junction Loop\again'
if (-not (Test-Path -LiteralPath $loop)) { New-Item -ItemType Junction -Path $loop -Target (Join-Path $media 'Junction Loop') | Out-Null }

Get-ChildItem -LiteralPath $media -File -Recurse | Measure-Object | ForEach-Object { "Fixtures ready in ${media}: $($_.Count) files" }
