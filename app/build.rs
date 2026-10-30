//! Genera el icono de la app (el mismo dibujo de `src/icon.rs`) y lo incrusta en el exe junto al
//! manifiesto y la información de versión.

#[path = "src/icon.rs"]
#[allow(dead_code)]
mod icon;

fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-changed=res/app.manifest");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let ico = out.join("app.ico");
    std::fs::write(&ico, icon::ico_file(&[16, 20, 24, 32, 40, 48, 64, 96, 128])).expect("escribir app.ico");

    let version = std::env::var("CARGO_PKG_VERSION").expect("versión");
    let nums: Vec<&str> = version.split('.').collect();
    let comma = format!("{},{},{},0", nums[0], nums.get(1).unwrap_or(&"0"), nums.get(2).unwrap_or(&"0"));
    let manifest =
        std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("dir")).join("res").join("app.manifest");
    let esc = |p: &std::path::Path| p.display().to_string().replace('\\', "\\\\");
    let rc = format!(
        r#"#include <winver.h>
1 ICON "{ico}"
1 24 "{manifest}"
VS_VERSION_INFO VERSIONINFO
FILEVERSION {comma}
PRODUCTVERSION {comma}
FILEOS VOS_NT_WINDOWS32
FILETYPE VFT_APP
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "Yesid Robayo"
      VALUE "FileDescription", "YesCharge"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "YesCharge"
      VALUE "LegalCopyright", "MIT License"
      VALUE "OriginalFilename", "YesCharge.exe"
      VALUE "ProductName", "YesCharge"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        ico = esc(&ico),
        manifest = esc(&manifest),
    );
    let rc_path = out.join("app.rc");
    std::fs::write(&rc_path, rc).expect("escribir app.rc");
    embed_resource::compile(&rc_path, embed_resource::NONE).manifest_required().expect("compilar recursos");
}
