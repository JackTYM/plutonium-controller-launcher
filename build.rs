fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/plutonium.ico");
        res.set("ProductName", "Plutonium Controller Launcher");
        res.set(
            "FileDescription",
            "Unofficial Plutonium updater replacement with controller navigation support",
        );
        res.set("CompanyName", "JackTYM");
        res.set("OriginalFilename", "plutonium.exe");
        res.set("InternalName", "plutonium");
        // Without an explicit manifest, Windows' installer-detection heuristic
        // flags this as needing elevation (likely triggered by the filename
        // "plutonium.exe" plus having no default "asInvoker" manifest once the
        // resource compiler replaces the linker's default one). Declare it
        // explicitly so it launches like any normal app.
        res.set_manifest(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false" />
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#,
        );
        res.compile().expect("failed to compile Windows resources");
    }
}
