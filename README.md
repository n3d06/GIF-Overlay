[![Download GIF-Overlay](https://img.shields.io/sourceforge/dt/gif-overlay.svg)](https://sourceforge.net/projects/gif-overlay/files/latest/download) [![Download GIF-Overlay](https://img.shields.io/sourceforge/dm/gif-overlay.svg)](https://sourceforge.net/projects/gif-overlay/files/latest/download)[![Download GIF-Overlay](https://img.shields.io/sourceforge/dw/gif-overlay.svg)](https://sourceforge.net/projects/gif-overlay/files/latest/download)[![Download GIF-Overlay](https://img.shields.io/sourceforge/dd/gif-overlay.svg)](https://sourceforge.net/projects/gif-overlay/files/latest/download)
# GIF Overlay - Minimalist GIF & Image Viewer

**GIF Overlay** is a lightweight, high-performance GIF and static image (PNG, JPG, BMP, TIFF, WebP) viewer for Windows, built with **Rust** and the native Win32 API. It focuses on minimalism, efficiency, and a premium aesthetic, allowing you to pin images on top of other windows with a unique translucency effect.

## 🎬 Demo Video

[▶ Watch the demo video](https://github.com/user-attachments/assets/86e83e9f-0311-4c02-abf9-e0740fbb4a06)


## ✨ Key Features

- **Native Rust + Win32:** A tiny (under 1 MB) single exe, no runtime required.
- **Single Instance:** Launching the exe again (or "Open With" on another file) reuses the running app: it brings the dashboard to the front, or opens the file in a new window.
- **History Dashboard:** Launching the app directly opens a dashboard of the images you opened before (empty on first run). Double-click an item to open it in a new window (you can have several open at once), `Add image` or drag one or more files in (up to 10 at once) to add them, press Delete or right-click to remove it. "Open With" skips the dashboard.
    - A File / View / Window / Help menu bar holds Add image, Paste, Clear history, Sort, the list of open images (with Minimize all, Restore all, Close all windows), How to use, Set as default viewer and Report a bug.
    - Help shows the current version and `Check for updates` compares it with the latest GitHub release (only when you click it; nothing is sent automatically).
    - Images that already have a window are marked with a green dot and "(open)"; opening one again brings its window to the front instead of duplicating it.
    - View menu: sort by recently opened, name (A–Z or Z–A) or file size (largest or smallest first); the choice is remembered.
    - `Clear history` empties the list after a confirmation (your image files are never touched).
- **Paste & Open from a link:** `Ctrl+V` opens a screenshot, a copied file or an `http(s)` image link from the clipboard in a new window. Pasted images are kept in a `pasted` folder next to the exe for as long as they are in the history.
- **Animated formats:** GIF, animated WebP and APNG play with their own frame timing; AVIF opens as a still image when the Windows AV1/AVIF codec is installed.
- **Always on Top:** Pin images over other applications—perfect for tracing, references, or desktop decoration.
- **Translucent Background:** Fully transparent window borders, showing only the image content.
- **Adjustable Opacity:** Slide to adjust transparency for a see-through effect.
- **Flexible Transforms:** 
    - Rotate Left/Right (90° increments).
    - Flip Horizontally/Vertically.
- **Dynamic Resizing:** 
    - Scale via Ctrl + Mouse Wheel.
    - Drag the corner (proportional) or the right/bottom edge (width or height only), or choose a preset scale (25%–300%) in the context menu.
- **High DPI Support:** Crisp rendering on all displays (4K, high-scale laptops).
- **Settings Persistence:** Remembers window opacity and lock state; the dashboard remembers your recent images.

## 🚀 How to Use

### Installation

**1. Via WinGet (Recommended)**
```bash
winget install gifoverlay
```

**2. Manual Download**
1. Download `GIF-Overlay-Setup.exe` from the [Releases](https://github.com/duyxyz/GIF-Overlay/releases) section.
2. Run the installer to register the application.
3. Right-click any image or `.gif` file -> **Open With** -> Select **GIF Overlay**.

### Controls
- **Move:** Left-click and drag the image (ensure window is unlocked).
- **Resize:** Hover the image to show the handles. Drag the corner to scale (keeps the aspect ratio), the right edge to change only the width, the bottom edge to change only the height. `Ctrl + Mouse Wheel` and the presets in Settings also work.
- **Context Menu (Right-Click):**
    - **Lock Window:** Prevent accidental moving or resizing (click-through; Ctrl+click to unlock).
    - **Dashboard:** Show the history dashboard to open a second (or third...) image in its own window. Every window has its own size, opacity, transforms and lock.
    - **Replace this image...:** Swap the image in this window for another GIF, PNG, JPG, BMP, TIFF, WebP, APNG or AVIF file.
    - **Paste image or link (Ctrl+V)**, **Copy image (Ctrl+C)** and **Show in folder.**
    - **Settings:** Scale presets, Opacity, Transform (Rotate, Flip) and Reset to Default.
    - **Always on Top**, **Pause / Play (Space)** (only for animations).
    - **Close window (Esc):** Close this window. Closing the last one quits unless the dashboard is showing.
    - **Quit GIF Overlay:** Close every window.
- **Keys:** `Esc` close, `Space` pause, `Up` / `Down` opacity, `Ctrl+C` copy the image, `Ctrl+V` paste, `Ctrl+O` add images.
- **Dashboard right-click:** on a thumbnail - Open, Show in folder, Copy path, Remove; on empty space - Add image, Paste.

## 🛠 Development

To build from source:

1. Install [Rust](https://rustup.rs) and the Visual Studio C++ Build Tools.
2. Clone the repository.
3. Build and run:
   ```bash
   cargo run --release -- src/assets/demo1.gif
   ```

## 📜 License
Released under the MIT License. Feel free to use and customize!

---
Developed by **DuyXYZ**

