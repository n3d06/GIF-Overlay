[![Download GIF-Overlay](https://img.shields.io/sourceforge/dt/gif-overlay.svg)](https://sourceforge.net/projects/gif-overlay/files/latest/download) [![Download GIF-Overlay](https://img.shields.io/sourceforge/dm/gif-overlay.svg)](https://sourceforge.net/projects/gif-overlay/files/latest/download)[![Download GIF-Overlay](https://img.shields.io/sourceforge/dw/gif-overlay.svg)](https://sourceforge.net/projects/gif-overlay/files/latest/download)[![Download GIF-Overlay](https://img.shields.io/sourceforge/dd/gif-overlay.svg)](https://sourceforge.net/projects/gif-overlay/files/latest/download)
# GIF Overlay - Minimalist GIF & Image Viewer

**GIF Overlay** is a lightweight, high-performance GIF and static image (PNG, JPG, BMP, TIFF, WebP) viewer for Windows, built with **Rust** and the native Win32 API. It focuses on minimalism, efficiency, and a premium aesthetic, allowing you to pin images on top of other windows with a unique translucency effect.

![GIF Overlay Demo](src/assets/demo1.gif)

## ✨ Key Features

- **Native Rust + Win32:** A tiny (~300 KB) single exe, no runtime required.
- **Single Instance:** Launching the exe again (or "Open With" on another file) reuses the running app: it brings the dashboard to the front, or opens the file in a new window.
- **History Dashboard:** Launching the app directly opens a dashboard of the images you opened before (empty on first run). Double-click an item to open it in a new window (you can have several open at once), `Add image...` or drag a file in to add one, press Delete or right-click to remove it. "Open With" skips the dashboard.
    - Images that already have a window are marked with a green dot and "(open)"; opening one again brings its window to the front instead of duplicating it.
    - Sort by recently opened or by name (A–Z); the choice is remembered.
    - `Clear history` empties the list after a confirmation (your image files are never touched).
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
    - **Lock Window:** Prevent accidental moving or resizing.
    - **Open another image...:** Show the history dashboard to open a second (or third...) image in its own window. Every window has its own size, opacity, transforms and lock.
    - **Settings:** Scale presets, Opacity and Transform (Rotate, Flip).
    - **Pause / Play:** Toggle GIF animation.
    - **Reset to Default:** Restore original size and transforms.
    - **Replace this image...:** Swap the image in this window for another GIF, PNG, JPG, BMP, TIFF or WebP file.
    - **Close:** Close this window. Closing the last one returns to the dashboard; close the dashboard window to quit.

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

