# crim — technical summary

Rationale behind the design, and where to plug new features. Version 0.2.0.

---

## 1. Guiding principle: the command is the document

The application never hides what it does. Every interaction (trim cursors,
crop rectangle, encoder settings) mutates a small data model, and that model is
serialised into a single `ffmpeg` command line displayed in an editable text
box. The **Export** button runs *that text*, not an internally rebuilt command.

Consequences:

- what you see is provably what runs — no divergence between UI and behaviour;
- the escape hatch is free: any ffmpeg feature not exposed in the UI can be
  typed by hand (`-vf` chains, `-c copy`, hardware encoders…);
- the model → command function (`command::build_command`) is a pure function,
  which is why it is the part covered by unit tests.

The generated text is quoted so it is valid POSIX shell *and* is parsed back
with `shell-words` before execution — one test asserts that round trip on a
path containing a space.

---

## 2. GUI: egui + eframe

Requirement was "cross-platform and well documented".

| Option | Verdict |
|---|---|
| **egui/eframe** | chosen |
| iced | good architecture, but a crop overlay requires implementing a custom widget with its own layout/draw/event plumbing |
| slint | adds a DSL; pixel-level pointer interaction is more awkward |
| GTK4 / Qt | native look, but heavy system dependencies and poor Windows/macOS story for a small tool |

Why egui fits this specific program:

- **Immediate mode is a natural fit for direct manipulation.** The crop
  rectangle and the timeline are drawn with a painter and hit-tested by hand,
  in ~200 lines each. There is no widget tree to synchronise with the model:
  the model *is* the source of truth, re-read every frame.
- **One static binary, no runtime.** Pure Rust, `glow` (OpenGL) backend, X11
  and Wayland both enabled, and the same code builds on macOS and Windows.
- **Documentation and stability.** egui has an extensive live demo whose source
  is the de-facto reference, and a large ecosystem of add-on crates.

Cost accepted: egui does not look native, and it redraws on demand — which is
fine because this app is idle most of the time (repaints are only requested
when a frame arrives, during a drag, or while exporting).

---

## 3. ffmpeg integration: process, not bindings

Two options existed: linking `libav*` through `ffmpeg-next`, or driving the
command line tools.

The CLI was chosen because:

1. **Build simplicity.** `ffmpeg-next` needs the ffmpeg development headers,
   pkg-config and matching library versions on every target OS; the FFI layer
   also drags `unsafe` code into an otherwise safe codebase. Here `cargo build`
   works with nothing but a Rust toolchain.
2. **Consistency with the deliverable.** The app *shows* an ffmpeg command; if
   the preview and the export used a different code path (in-process decoding
   vs the CLI), the two could drift. Same binary, same filters, same result.
3. **Robustness.** A malformed file crashes a child process, not the editor.

The price is one process spawn per operation (~20–60 ms per preview frame),
which is handled by the throttling and coalescing described below. If frame
accuracy at 4K ever becomes the bottleneck, `frames.rs` is the single module to
swap for a libav-based decoder: its public API (`FrameRequest` in,
`DecodedFrame` out) does not mention ffmpeg.

Three ffmpeg entry points are used:

| Task | Tool | Command shape |
|---|---|---|
| Metadata | `ffprobe` | `-print_format json -show_format -show_streams` |
| Preview frame | `ffmpeg` | `-ss t -i f -frames:v 1 -vf scale=960:-2 -f image2pipe -vcodec png -` |
| Playback + audio | `ffplay` | `-ss in -t dur -vf crop=… f` |
| Export | `ffmpeg` | `-y -ss in -t dur -i f -vf crop=… -c:v libx264 -crf … out` |

Notable details:

- **Options are omitted rather than neutralised.** No `-ss` when the in point
  is 0, no `-t` at full length, no `crop=` on a full frame, and no `-crf`,
  `-preset`, `-pix_fmt` or `-b:a` when the corresponding stream is copied
  (ffmpeg rejects encoder options without an encoder). The UI hides those
  controls in the same conditions, so the widgets and the command can never
  disagree.
- **Muting is `-an`, not a volume filter**, and it is applied to the `ffplay`
  preview as well: the preview must be the export, minus the encoding.
- **`-ss` before `-i`** uses demuxer-level seeking (fast) and has been frame
  accurate since ffmpeg 2.1, because the decoder replays from the previous
  keyframe. Placing it after `-i` would decode the whole prefix.
- **PNG over a pipe** avoids temporary files and gives a lossless, easily
  decoded RGBA buffer; the frame is downscaled to 960 px first, so decoding and
  the GPU upload stay cheap regardless of the source resolution.
- **Crop forces a re-encode**, so `-c copy` is never generated; `-pix_fmt
  yuv420p` and even crop dimensions are enforced because 4:2:0 chroma cannot
  represent odd sizes and libx264 would abort.
- **Playback is delegated to `ffplay`.** Implementing A/V playback in-process
  means decoding, resampling, an audio device, and a synchronisation clock —
  more code than the rest of the application combined, for a preview. `ffplay`
  accepts the same `-ss/-t/-vf` options, so what is played is what is exported.
  While it runs, the playhead is advanced from the wall clock (`Instant`),
  which is accurate enough for a cursor and costs nothing.

---

## 4. Concurrency model

The GUI thread must never block. Three isolated pieces of asynchrony, all built
on `std::sync::mpsc` plus a repaint callback — no async runtime, no `Arc<Mutex>`
around the application state:

1. **Frame extractor** (`frames.rs`): a dedicated thread that blocks on
   `recv()`. When it wakes up it **drains the queue and keeps only the newest
   request**, so dragging the playhead never builds a backlog of stale frames.
   Results come back on a second channel; the worker calls
   `egui::Context::request_repaint()` to wake the UI.
2. **Throttling** (`app.rs`): requests are additionally rate-limited to one per
   70 ms. Coalescing alone would still spawn a process per mouse move on a fast
   machine; the throttle bounds the process spawn rate, and the two combined
   give a smooth scrub. Frame stepping bypasses the throttle for instant
   feedback.
3. **Export job** (`command.rs`): the child's stderr is read on its own thread
   and split on both `\r` and `\n` (ffmpeg overwrites its progress line with a
   carriage return, so `BufRead::lines()` would show nothing until the end).
   The last status line is forwarded to the UI; the child handle is kept behind
   an `Arc<Mutex<…>>` only so that **Cancel** and `on_exit` can kill it.

---

## 5. Interaction details worth knowing

A few decisions in the widgets come from actual use rather than from theory:

- **The playhead outranks the trim cursors.** When they overlap, a drag always
  grabs the playhead: it is the control the user moves continuously, whereas
  trim points are set once and have keyboard-free alternatives (**Set in** /
  **Set out**). Without this rule, the in point becomes impossible to leave once
  the playhead sits on it. The rule lives in one `if` at the top of
  `timeline.rs`, before the nearest-handle search.
- **Fixed geometry constants, not layout guessing.** The timeline reserves
  explicit bands (`TRACK_TOP`, `TRACK_HEIGHT`, `LABEL_SIZE`) and the widget
  height is the sum of them, so labels cannot be clipped by the panel. Labels
  are additionally clamped inside the widget rectangle instead of being centred
  blindly on the cursor.
- **Free-form text fields were replaced by dropdowns** for the codecs, with a
  `custom…` entry that reveals a text field. A closed list of the common cases
  removes typos in the frequent path while `custom…` keeps every ffmpeg encoder
  reachable — the same "sane default, no ceiling" principle as the editable
  command box. It is a single reusable helper, `ui::codec_combo`.
- **Information text is visually subordinate.** The status line (file, decoded
  frame, crop, selection) is monospaced at 9.5 pt and weak-coloured: it must be
  readable when looked for, never compete with the controls.
- **The icon is embedded** with `include_bytes!` and decoded at startup, so the
  binary stays self-contained; the Linux desktop entry only matters for the dock
  and must share the app id set in `main.rs`.

---

## 6. Data model choices

- **Crop stored normalised (0..1)**, converted to pixels only when the command
  is built. It is therefore independent of the preview scale, of the window
  size and of the source resolution — the widget maps screen ↔ normalised
  coordinates and nothing else knows about pixels. Rounding to even pixel
  values happens in one place, `Crop::to_pixels`.
- **Trim points in `f64` seconds**, formatted as `HH:MM:SS.mmm` for ffmpeg.
  The minimum gap between the two cursors is one frame (`1/fps` from ffprobe).
- **`state.rs` has no dependency on egui or on `std::process`.** The model can
  be reused by a CLI or a batch mode without touching the GUI.

---

## 7. Module boundaries (where to extend)

| Want to add | Touch |
|---|---|
| A new transform (rotate, scale, fade, speed) | add a field in `state.rs`, one filter in `build_command`, one control in `app.rs` |
| A different encoder (NVENC, VAAPI, ProRes) | `ExportSettings` + `build_command`; nothing else |
| Multiple trim segments | turn `start`/`end` into a `Vec<Segment>`; `timeline.rs` iterates over it, `build_command` emits a `concat` filter |
| Faster / smoother scrubbing | replace the body of `frames.rs` (keep the channel API), e.g. keep one long-running ffmpeg pipe or link libav |
| Waveform or thumbnail strip in the timeline | new painter calls in `timeline.rs`, fed by a second worker modelled on `frames.rs` |
| Presets / recent files | serialise `EditState` with serde; `eframe` already offers persistent storage via `App::save` |

The widget contract is uniform and stateless-by-default: each widget takes
`&mut` model plus its own drag state, and returns what changed
(`TimelineChange { seek, trim }`, `bool` for the crop). The application decides
what to do with that — rebuild the command, request a frame, or both. Adding a
widget means following the same three-line pattern in `app.rs`.

---

## 8. Testing

`cargo test` covers the deterministic core (8 tests):

- command generation with and without trim/crop;
- muting (`-an` replaces the audio options);
- stream copy (no encoder options are emitted);
- even-pixel rounding and clamping of the crop rectangle;
- the display ↔ `shell-words` round trip on a path containing a space;
- an end-to-end frame extraction: ffmpeg synthesises a `testsrc` clip, the
  worker decodes a frame from it and the RGBA buffer size is verified (skipped
  automatically when ffmpeg is absent).

The GUI itself is not tested; keeping all the non-visual logic outside the
widgets is what makes the tested surface meaningful.
