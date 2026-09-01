# crim — **cr**op & tr**im**

A small cross-platform desktop application (Rust + egui) that trims and crops a
video visually and shows, at all times, the exact `ffmpeg` command that will
produce the result. The command is editable, and **Export** runs precisely the
text you see.

- Version 0.2.0
- Documentation: [`docs/FEATURES.md`](docs/FEATURES.md) (what it does today),
  [`docs/tech_summary.md`](docs/tech_summary.md) (why it is built this way, and
  where to extend it), [`docs/CHANGELOG.md`](docs/CHANGELOG.md).

---

## 1. Requirements

| Component | Minimum version | Why |
|---|---|---|
| Rust toolchain (`cargo`, `rustc`) | 1.76 (stable) | builds the application |
| `ffmpeg` | 4.x | extracts preview frames, encodes the output |
| `ffprobe` | 4.x | reads resolution / duration / fps (ships with ffmpeg) |
| `ffplay` | 4.x | preview playback with audio (ships with ffmpeg; optional) |

The three binaries must be reachable through `PATH`. The application checks
this at startup: a red banner appears if `ffmpeg` is missing, and **Play** is
disabled when `ffplay` is missing.

### Install the Rust toolchain

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

### Install ffmpeg

```bash
# Debian / Ubuntu / Pop!_OS
sudo apt update && sudo apt install -y ffmpeg

# Fedora (ffplay lives in the RPM Fusion build)
sudo dnf install -y ffmpeg

# Arch
sudo pacman -S ffmpeg

# macOS
brew install ffmpeg

# Windows
winget install Gyan.FFmpeg
```

Verify:

```bash
ffmpeg -version && ffprobe -version && ffplay -version
```

### Extra system libraries (Linux only)

`eframe` uses the OpenGL backend, plus X11/Wayland and the XDG portal for the
file dialog. On a desktop installation these are already present; on a minimal
system install the development packages:

```bash
# Debian / Ubuntu / Pop!_OS
sudo apt install -y build-essential pkg-config \
    libx11-dev libxcursor-dev libxrandr-dev libxi-dev \
    libxkbcommon-dev libwayland-dev libgl1-mesa-dev

# Fedora
sudo dnf install -y gcc pkgconf-pkg-config libX11-devel libXcursor-devel \
    libXrandr-devel libXi-devel libxkbcommon-devel wayland-devel mesa-libGL-devel
```

macOS and Windows need no extra packages beyond the toolchain
(Xcode command line tools / MSVC build tools).

---

## 2. Build

```bash
cd crim
cargo build --release
```

The binary is `target/release/crim` (`crim.exe` on Windows). Optionally install
it into `~/.cargo/bin` (already in `PATH` after rustup):

```bash
cargo install --path .
```

Run the test suite (unit tests for the command generator plus one end-to-end
test that generates a clip with ffmpeg and decodes a frame from it):

```bash
cargo test
```

### Desktop entry (Linux, optional)

The window icon is embedded in the binary. To also get it in the dock and in
the application menu, install the provided desktop entry:

```bash
install -Dm755 target/release/crim ~/.local/bin/crim
install -Dm644 assets/icon.png ~/.local/share/icons/hicolor/256x256/apps/crim.png
install -Dm644 assets/crim.desktop ~/.local/share/applications/crim.desktop
update-desktop-database ~/.local/share/applications 2>/dev/null || true
```

---

## 3. Run

```bash
crim                  # then use "Open video…"
crim clip.mp4         # open a file directly
```

You can also drag and drop a video file onto the window.

---

## 4. Usage

### Timeline (bottom of the window)

- The two light handles are the trim cursors (`in` / `out`).
- The red cursor is the playhead, with the current timestamp underneath. Drag
  it, or click anywhere on the track to move it there. It is always kept inside
  the trimmed region.
- When the playhead and a trim cursor overlap, dragging always grabs the
  **playhead**; use **Set in** / **Set out** to move a trim point to the
  playhead position instead.
- `⏴` / `⏵` step exactly one frame backwards/forwards.

### Preview (centre)

- The orange rectangle is the crop. Drag its 8 handles to resize, drag the
  inside to move it. Everything that will be discarded is darkened, and
  rule-of-thirds guides are drawn inside the kept area.
- **Reset crop** restores the full frame.

### Playback

- **Play** opens an `ffplay` window that plays only the selected range, with
  the crop applied and with audio — i.e. exactly what will be exported. The
  playhead follows along.
- **Stop** (or closing the `ffplay` window) ends playback.
- **Audio / Muted** toggles `-an`: the audio stream is dropped from the export
  *and* from the preview. Disabled for files with no audio track. Toggling it
  while playing restarts the preview so it stays consistent.

### Encoding options

- **video**: encoder dropdown (H.264, H.265, VP9, AV1, NVENC, VAAPI, `copy`,
  or `custom…` to type any ffmpeg encoder name).
- **crf** / **preset**: quality and speed; hidden when the codec is `copy`,
  which takes no encoder options.
- **audio**: encoder dropdown (AAC, MP3, Opus, AC3, FLAC, PCM, `copy`,
  `custom…`) and a bitrate dropdown.
- **Output…**: destination file (defaults to `<name>_edit.mp4` next to the
  source).

Selecting `copy` for the video while a crop is active shows a warning: a stream
copy cannot apply a filter.

### Command box

- Always shows the command corresponding to the current selection.
- It is editable: as soon as you type in it, it stops being regenerated (an
  "edited by hand" marker appears, with **Regenerate** to return to automatic
  mode).
- **Copy** puts it in the clipboard. **Export** runs exactly the text shown,
  reporting ffmpeg's progress line and its exit status; **Cancel** kills it.

---

## 5. Troubleshooting

| Symptom | Cause / fix |
|---|---|
| Red banner "ffmpeg/ffprobe not found" | ffmpeg is not in `PATH`. On Windows, reopen the terminal after installing. |
| **Play** greyed out | `ffplay` missing. Some distributions split it out; on Fedora it requires the RPM Fusion ffmpeg build. |
| "no frame" in the preview | The playhead sits past the last decodable frame, or the stream cannot be decoded at that position. |
| Export fails with `height/width not divisible by 2` | Should not happen (crop dimensions are rounded to even), unless the command was edited by hand. |
| Export fails with `Filtergraph … not supported with copy` | `-c:v copy` cannot crop: choose an encoder or reset the crop. |
| Slow scrubbing on 4K files | Each seek spawns one ffmpeg process. Raise `SEEK_THROTTLE` or lower `PREVIEW_WIDTH` in `src/app.rs`. |
| The file dialog does not open on Linux | Install `xdg-desktop-portal` and a backend (`xdg-desktop-portal-gtk` or `-kde`). |
| No icon in the dock (Wayland) | Install the desktop entry as shown above; the app id (`crim`) must match `StartupWMClass`. |

---

## 6. Project layout

```
assets/
├── icon.png         application icon (embedded in the binary)
└── crim.desktop     Linux desktop entry
docs/
├── FEATURES.md      current feature list
├── tech_summary.md  technical choices and extension points
└── CHANGELOG.md     version history
src/
├── main.rs        entry point, window and icon creation
├── app.rs         application state, panel layout, glue between the modules
├── state.rs       data model: trim points, crop rectangle, encoder settings
├── ffprobe.rs     metadata reading (resolution, duration, fps, audio)
├── frames.rs      background worker extracting preview frames via ffmpeg
├── command.rs     ffmpeg command generation + execution of the export
├── player.rs      playback with audio through ffplay
└── ui/
    ├── mod.rs       colour palette and the codec dropdown helper
    ├── timeline.rs  timeline with the two trim cursors and the playhead
    └── preview.rs   frame display with the interactive crop rectangle
```
