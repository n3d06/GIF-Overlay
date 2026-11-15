fn main() {
    winresource::WindowsResource::new()
        .set_icon("src/assets/app_icon.ico")
        .set("ProductName", "GIF Overlay")
        .set("FileDescription", "Minimalist GIF & Image Viewer")
        .compile()
        .unwrap();
}
