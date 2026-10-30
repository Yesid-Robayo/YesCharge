//! Rutas de datos: todo vive en `%APPDATA%\YesCharge`, salvo que exista una carpeta `YesChargeData`
//! junto al exe (modo portable) o se defina `YESCHARGE_DATA_DIR` (útil para pruebas).

use std::path::PathBuf;

/// Dónde estarían los datos en modo portable.
pub fn portable_dir() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join("YesChargeData"))
}

pub fn is_portable() -> bool {
    portable_dir().is_some_and(|d| d == data_dir())
}

pub fn data_dir() -> PathBuf {
    // las pruebas automáticas nunca deben tocar los datos reales del usuario
    #[cfg(test)]
    let dir = std::env::temp_dir().join("yescharge-tests");
    #[cfg(not(test))]
    let dir = match std::env::var_os("YESCHARGE_DATA_DIR") {
        Some(d) => PathBuf::from(d),
        None => portable_dir().filter(|d| d.is_dir()).unwrap_or_else(|| {
            std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("YesCharge")
        }),
    };
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn config_file() -> PathBuf {
    data_dir().join("config.json")
}

pub fn status_file() -> PathBuf {
    data_dir().join("status.json")
}

pub fn history_file() -> PathBuf {
    data_dir().join("history.json")
}

pub fn model_file() -> PathBuf {
    data_dir().join("model.json")
}

pub fn state_file() -> PathBuf {
    data_dir().join("state.json")
}

pub fn log_file() -> PathBuf {
    data_dir().join("yescharge.log")
}

pub fn captures_dir() -> PathBuf {
    let dir = data_dir().join("captures");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Escribe un archivo de forma atómica (temporal + renombrar), para que otro proceso nunca
/// lea un JSON a medias.
pub fn write_atomic(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    #[test]
    fn tests_never_use_the_real_data_folder() {
        assert!(super::data_dir().starts_with(std::env::temp_dir()));
    }
}
