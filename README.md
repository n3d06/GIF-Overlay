[![Download GIF-Overlay](https://img.shields.io/sourceforge/dt/gif-overlay.svg)](https://sourceforge.net/projects/gif-overlay/files/latest/download) [![Download GIF-Overlay](https://img.shields.io/sourceforge/dm/gif-overlay.svg)](https://sourceforge.net/projects/gif-overlay/files/latest/download)[![Download GIF-Overlay](https://img.shields.io/sourceforge/dw/gif-overlay.svg)](https://sourceforge.net/projects/gif-overlay/files/latest/download)[![Download GIF-Overlay](https://img.shields.io/sourceforge/dd/gif-overlay.svg)](https://sourceforge.net/projects/gif-overlay/files/latest/download)
# GIF Overlay - Minimalist GIF & Image Viewer

**GIF Overlay** is a lightweight, high-performance GIF and static image viewer for Windows, built with **Rust** and the native Win32 API. It focuses on minimalism, efficiency, and a premium aesthetic, allowing you to pin images on top of other windows with a unique translucency effect.

![GIF Overlay Demo](src/assets/demo1.gif)

## ✨ Key Features

- **Native Rust + Win32:** A tiny (~300 KB) single exe, no runtime required.
- **History Dashboard:** Launching the app directly opens a dashboard of the GIFs you opened before (empty on first run). Double-click an item to show it, `Add GIF...` or drag a file in to add one, press Delete or right-click to remove it. "Open With" skips the dashboard.
- **Always on Top:** Pin images over other applications—perfect for tracing, references, or desktop decoration.
- **Translucent Background:** Fully transparent window borders, showing only the image content.
- **Adjustable Opacity:** Slide to adjust transparency for a see-through effect.
- **Flexible Transforms:** 
    - Rotate Left/Right (90° increments).
    - Flip Horizontally/Vertically.
- **Dynamic Resizing:** 
    - Scale via Ctrl + Mouse Wheel.
    - Preset scale levels (25%–300%) in the context menu.
- **High DPI Support:** Crisp rendering on all displays (4K, high-scale laptops).
- **Settings Persistence:** Remembers your last opened GIF, window size, and opacity.

## 🚀 How to Use

### Installation

**1. Via WinGet (Recommended)**
```bash
winget install gifoverlay
```

**2. Manual Download**
1. Download `GIF-Overlay-Setup.exe` from the [Releases](https://github.com/duyxyz/GIF-Overlay/releases) section.
2. Run the installer to register the application.
3. Right-click any `.gif` file -> **Open With** -> Select **GIF Overlay**.

### Controls
- **Move:** Left-click and drag the image (ensure window is unlocked).
- **Resize:** `Ctrl + Mouse Wheel` to scale or pick a preset in Settings.
- **Context Menu (Right-Click):**
    - **Lock Window:** Prevent accidental moving or resizing.
    - **Change GIF...:** Select a different file.
    - **Settings:** Access Resizing (Scale, W, H, Opacity) and Transform (Rotate, Flip).
    - **Pause / Play:** Toggle GIF animation.
    - **Reset to Default:** Restore original size and transforms.
    - **History...:** Open the dashboard while keeping the current GIF.
    - **Close (back to history):** Close the overlay and return to the dashboard. Close the dashboard window to quit.

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

