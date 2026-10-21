# Analytics

ReVLC sends usage statistics, with no personal information, to PostHog for
one reason: **to know which versions and machines it runs on, and whether
people get a good viewing experience** (playback that starts, starts quickly, seeks quickly and doesn't
stall). Anything that doesn't serve that question isn't collected.

- Each install gets a random id (`analytics.json` in the app's data folder).
  There are no accounts, no person profiles, and nothing that identifies the user.
- Nothing about what's watched leaves the machine: no file names, titles,
  paths, URLs or hosts. A source is described only by its kind (`local`,
  `network`, `disc`, `capture`), its scheme (`file`, `https`, `rtsp`...) and,
  if it's a known one, its container extension (`mkv`, `m3u8`...; anything else is `other`).
- It can be turned off in Settings → General → Privacy. Off, nothing is sent or queued.
- The first-run screen says so.
- Development builds print events to the terminal (`revlc: analytics: ...`)
  instead of sending them. Set `REVLC_ANALYTICS_SEND=1` to send them anyway.

## Setup

The project is "ReVLC" (id 308872) in PostHog's EU region. Its project key
is in `PROJECT_KEY` in `src-tauri/src/telemetry.rs` and `POSTHOG_KEY` in
`site/analytics.js`. The key only allows sending events, so it's fine to
commit.

In the project settings, decide on IP addresses. PostHog uses them to add
the country. To keep no IPs at all, turn on "Discard client IP data".

## Events

Every app event carries: `$app_version`, `$os` (`Mac OS X` / `Windows`),
`$os_version` (`15.2`; `10` or `11`), `os_build`, `arch`, `cpu`
(`Apple M2`), `cpu_cores`, `ram_gb`, `machine_model` (`Mac14,2` or the PC's
product name), `libvlc_version`.

| Event | When | Properties |
|---|---|---|
| `app_installed` | First launch | |
| `app_updated` | First launch of a new version | `from_version` |
| `app_launched` | Every launch | |
| `app_active` | Once a day while the app stays open, so long sessions count as active | |
| `engine_failed` | libVLC couldn't load: nothing can play | |
| `playback_started` | First frame on screen | `ttff_ms`, plus the media properties below |
| `playback_failed` | Never reached a first frame | `stage` (`open`, `error`, `ended_before_playing`), `elapsed_ms` |
| `playback_abandoned` | Stopped or replaced before the first frame | `reason`, `waited_ms` |
| `seek_completed` | Playback resumed after a seek | `latency_ms`, `kind`, `distance_ms`, `direction`, `rate` |
| `seek_failed` | No resume after 60 s, or closed while waiting | `reason` (`timeout`, `closed`), `waited_ms`, `kind` ... |
| `playback_ended` | Finished, stopped, replaced, error, quit | `reason`, `watched_ms`, `stalls`, `stall_ms`, `seeks`, `superseded_seeks` |
| `download_clicked` (site) | A download button was clicked | `platform`, `version`, `arch`, `file`, `placement` |

Playback events share `playback_id` and these properties: `source`,
`scheme`, `format`, `resumed`, `hw_decoding` (the setting), `caching_ms` (the
file or network caching setting that applied). From the first frame on, they
also have `has_video`, `video_codec` (`h264`, `hevc`, `av01`...),
`audio_codec`, `width`, `height`, `resolution` (`720p`, `1080p`, `4K`...),
`fps`, `live` and `duration_s`.

Seek `kind`: `jump` (seek bar), `skip` (arrow keys / ±10 s), `chapter`,
`resume` (accepting "continue where you left off"), `loop` (A–B loop). Seeks
while paused aren't measured, as playback only resumes when the user says so.
A seek replaced by another before it finished isn't measured either; it's
counted in `superseded_seeks`.

### How the timings are measured

All in `src-tauri/src/quality.rs`, from libVLC's own events, timestamped
when libVLC sends them:

- **Time to first frame**: from opening the item to libVLC creating the
  video output, which it does for the first decoded frame. Audio-only media
  uses the first time update. Resuming from a saved point includes that seek.
  Measured: ~300 ms for a local 1080p H.264 file, ~1.5 s for an HLS stream.
- **Seek latency**: from the request to playback resuming. libVLC reports a
  seek by dropping its buffer and filling it again, then sending time updates
  again. The first time update after that, less any media time already played
  past the target, is when playback resumed. Measured: 50–750 ms locally
  (precise seeks decode from the previous keyframe), ~10 s on a slow HLS stream.
- **Stalls**: the buffer running dry mid-playback (not after a seek or
  pause), until it refills.

## The metrics

PostHog → SQL insights (HogQL). Each can be saved to a dashboard.

**Downloads**: `download_clicked` counts clicks; GitHub counts actual
downloads (`scripts/download-stats.sh`). New installs that ran:
`app_installed`.

```sql
SELECT properties.platform AS platform, properties.version AS version, count() AS clicks
FROM events
WHERE event = 'download_clicked' AND timestamp > now() - INTERVAL 30 DAY
GROUP BY platform, version ORDER BY clicks DESC
```

**Active users** by platform and version: a Trends insight, "Unique users"
of any event, broken down by `$os` then `$app_version`, daily or weekly. Or:

```sql
SELECT toDate(timestamp) AS day, properties.$os AS os, properties.$app_version AS version,
  count(DISTINCT distinct_id) AS users
FROM events
WHERE timestamp > now() - INTERVAL 30 DAY
GROUP BY day, os, version ORDER BY day
```

**Successful playback rate**: of playbacks that ended one way or the other
(started or failed), the share that started. Abandoned ones are left out, as
the user didn't wait to find out, but a rising abandon rate is worth watching.

```sql
SELECT properties.$app_version AS version, properties.source AS source,
  countIf(event = 'playback_started') AS started,
  countIf(event = 'playback_failed') AS failed,
  countIf(event = 'playback_abandoned') AS abandoned,
  round(100 * started / nullIf(started + failed, 0), 2) AS success_pct
FROM events
WHERE event IN ('playback_started', 'playback_failed', 'playback_abandoned')
  AND timestamp > now() - INTERVAL 7 DAY
GROUP BY version, source ORDER BY version DESC
```

**Time to first frame**, P50 / P95:

```sql
SELECT properties.source AS source, properties.$os AS os, count() AS plays,
  quantile(0.5)(toFloat(properties.ttff_ms)) AS p50,
  quantile(0.95)(toFloat(properties.ttff_ms)) AS p95
FROM events
WHERE event = 'playback_started' AND timestamp > now() - INTERVAL 7 DAY
GROUP BY source, os ORDER BY plays DESC
```

**Seek latency**, P50 / P95, local and streamed apart:

```sql
SELECT properties.source AS source, properties.format AS format, count() AS seeks,
  quantile(0.5)(toFloat(properties.latency_ms)) AS p50,
  quantile(0.95)(toFloat(properties.latency_ms)) AS p95
FROM events
WHERE event = 'seek_completed' AND properties.kind IN ('jump', 'skip', 'chapter')
  AND timestamp > now() - INTERVAL 7 DAY
GROUP BY source, format ORDER BY seeks DESC
```

Seeks that never finished are in `seek_failed`; count them next to these, or
the P95 looks better than it is.

**Rebuffering ratio** (time stalled per time watched), for streams:

```sql
SELECT properties.source AS source,
  sum(toFloat(properties.stall_ms)) / nullIf(sum(toFloat(properties.watched_ms)) + sum(toFloat(properties.stall_ms)), 0) AS rebuffer_ratio,
  avg(toFloat(properties.stalls)) AS stalls_per_playback
FROM events
WHERE event = 'playback_ended' AND timestamp > now() - INTERVAL 7 DAY
GROUP BY source
```

Break any of these down by `$app_version`, `video_codec`, `resolution`,
`hw_decoding`, `cpu` or `$os_version` to find where the experience is worse.
