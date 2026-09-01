# crim — feature list (0.2.0)

Status of every feature currently implemented, with the module that owns it.
Use this as the checklist to keep updated when extending the application.

---

## File handling

| Feature | Where | Notes |
|---|---|---|
| Open via file dialog | `app.rs` (`pick_file`) | native dialog through `rfd`; filters common video extensions |
| Open via command line | `main.rs` | `crim clip.mp4` |
| Open via drag & drop | `app.rs` (`update`) | first dropped file wins |
| Metadata reading | `ffprobe.rs` | width, height, duration, average fps, presence of an audio stream |
| Failure reporting | `app.rs` | missing binary, no video stream, unreadable resolution → red message in the status bar |
| Startup dependency check | `command.rs` (`binary_available`) | banner if `ffmpeg`/`ffprobe` missing, **Play** disabled if `ffplay` missing |

## Preview

| Feature | Where | Notes |
|---|---|---|
| Still-frame preview at the playhead | `frames.rs` | `ffmpeg -ss t … -f image2pipe -vcodec png -`, decoded to RGBA |
| Downscaled decoding | `app.rs` (`PREVIEW_WIDTH = 960`) | keeps decode + GPU upload cheap on 4K sources |
| Non-blocking extraction | `frames.rs` | dedicated thread; newest request wins, older ones are dropped |
| Request throttling | `app.rs` (`SEEK_THROTTLE = 70 ms`) | bounds the process spawn rate while dragging; bypassed for frame stepping |
| Texture reuse | `app.rs` (`accept_frame`) | one GPU texture, updated in place |
| Letterboxed display | `ui/preview.rs` | aspect ratio always preserved inside the panel |
| Timestamp of the displayed frame | toolbar (right) | may briefly lag the playhead while decoding |

## Trimming

| Feature | Where | Notes |
|---|---|---|
| Two draggable trim cursors | `ui/timeline.rs` | in/out, with their timestamps above the track |
| Draggable playhead | `ui/timeline.rs` | clamped inside `[in, out]`; timestamp shown underneath |
| Playhead priority on overlap | `ui/timeline.rs` | when the playhead and a trim cursor coincide, the drag grabs the playhead |
| Click-to-seek | `ui/timeline.rs` | clicking the track moves the playhead there |
| Frame stepping | toolbar `⏴` / `⏵` | one frame = `1/fps` from ffprobe |
| Set in / Set out | toolbar | snaps a trim cursor to the playhead |
| Minimum selection | `ui/timeline.rs` | the two cursors can never come closer than one frame |

## Cropping

| Feature | Where | Notes |
|---|---|---|
| Crop rectangle over the preview | `ui/preview.rs` | 8 resize handles + drag inside to move |
| Discarded area darkened | `ui/preview.rs` | four mask rectangles |
| Rule-of-thirds guides | `ui/preview.rs` | drawn inside the kept area |
| Resolution-independent model | `state.rs` (`Crop`) | stored normalised (0..1), converted to pixels only at export |
| Even-pixel rounding | `state.rs` (`Crop::to_pixels`) | required by 4:2:0 chroma / libx264 |
| Clamping | `state.rs` (`Crop::clamp`) | never leaves the frame, never smaller than 2 % |
| Reset crop | toolbar | back to the full frame |
| Live pixel readout | toolbar (right) | `crop W×H+X+Y` |

## Playback

| Feature | Where | Notes |
|---|---|---|
| Play the selection with audio | `player.rs` | `ffplay -ss … -t … [-vf crop=…] [-an]`, so it matches the export |
| Playhead follows playback | `player.rs` (`update`) | derived from the wall clock |
| Auto-stop at the out point | `player.rs` | also detects the user closing the ffplay window |
| Stop | toolbar | kills the child process; also on application exit |
| Mute toggle (`-an`) | toolbar + `command.rs` | applies to both the export and the preview; disabled when the source has no audio; restarts a running preview |

## Command generation

| Feature | Where | Notes |
|---|---|---|
| Live ffmpeg command | `command.rs` (`build_command`) | rebuilt on every change; pure function of the model |
| Fast, accurate trim | `command.rs` | `-ss`/`-t` before `-i`, frame accurate because the output is re-encoded |
| Options omitted when irrelevant | `command.rs` | no `-ss` at 0, no `-t` at full length, no `crop=` on a full frame |
| Stream copy support | `command.rs` (`is_copy`) | `-c:v copy` / `-c:a copy` drop `-crf`, `-preset`, `-pix_fmt`, `-b:a` |
| Shell-safe quoting | `command.rs` (`quote`) | the displayed line can be pasted into a terminal as is |
| Editable command | `app.rs` | typing switches to manual mode; **Regenerate** returns to automatic |
| Copy to clipboard | `app.rs` | |

## Encoding options

| Feature | Where | Notes |
|---|---|---|
| Video encoder dropdown | `ui/mod.rs` (`codec_combo`) | libx264, libx265, libvpx-vp9, libsvtav1, mpeg4, h264_nvenc, h264_vaapi, copy, `custom…` |
| Audio encoder dropdown | same | aac, libmp3lame, libopus, ac3, flac, pcm_s16le, copy, `custom…` |
| Audio bitrate dropdown | same | 96k…320k, or a custom value |
| CRF | toolbar row | 0–51, hidden for `copy` |
| x264/x265 preset | toolbar row | ultrafast…veryslow, hidden for `copy` |
| Fixed `-pix_fmt yuv420p` | `command.rs` | maximum player compatibility |
| Output path | `app.rs` | defaults to `<name>_edit.mp4`, changeable with **Output…** |
| Crop + `copy` warning | `app.rs` | a stream copy cannot apply a filter |

## Export

| Feature | Where | Notes |
|---|---|---|
| Runs the displayed text | `command.rs` (`ExportJob`) | parsed with `shell-words`, so hand edits are honoured |
| Live progress | `command.rs` | ffmpeg's status line, read from stderr split on `\r` and `\n` |
| Result reporting | `app.rs` | ✔ on success, ✖ with the last stderr lines on failure |
| Cancel | `app.rs` / `ExportJob::cancel` | kills the encoder; also on application exit |
| Non-blocking | `command.rs` | stderr is read on its own thread; the GUI never waits |

## Application

| Feature | Where | Notes |
|---|---|---|
| Cross-platform | `Cargo.toml` | Linux (X11 + Wayland), macOS, Windows; no C dependency to build |
| Embedded icon | `main.rs` (`load_icon`) | `assets/icon.png`, plus `assets/crim.desktop` for Linux docks |
| Dark theme | `app.rs` | palette centralised in `ui/mod.rs::theme` |
| On-demand repainting | `app.rs` | repaints are requested by the workers, not on a fixed timer |
| Clean shutdown | `app.rs` (`on_exit`) | stops `ffplay` and any running export |
| Test suite | `command.rs`, `frames.rs` | 8 tests: command generation, crop rounding, quoting round trip, real frame extraction |

---

## Not implemented yet

Kept here so the roadmap is explicit; `docs/tech_summary.md` §7 maps each of
these to the files it would touch.

- Frame-exact in-window playback (currently delegated to a separate `ffplay` window).
- Multiple trim segments / cut list.
- Audio waveform or thumbnail strip in the timeline.
- Rotation, scaling, fades, speed changes.
- Presets, recent files, session persistence.
- Keyboard shortcuts.
- Batch processing of several files.
