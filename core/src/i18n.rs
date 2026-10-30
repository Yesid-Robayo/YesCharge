//! Idioma de la interfaz. Cada texto va en el sitio donde se usa, en los dos idiomas (`tr` y `trf!`):
//! así ninguno se queda sin traducir y el compilador comprueba los datos de cada formato. Los logs
//! no se traducen.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    /// `auto` es como se guardaba la opción "el de Windows", que ya no existe.
    #[default]
    #[serde(alias = "auto")]
    Spanish,
    English,
}

static ENGLISH: AtomicBool = AtomicBool::new(false);

/// Aplica el idioma a todo el proceso.
pub fn set(language: Language) {
    ENGLISH.store(language == Language::English, Ordering::Relaxed);
}

pub fn is_english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

pub fn tr(es: &'static str, en: &'static str) -> &'static str {
    if is_english() { en } else { es }
}

/// `format!` en los dos idiomas.
#[macro_export]
macro_rules! trf {
    ($es:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if $crate::i18n::is_english() { format!($en $(, $arg)*) } else { format!($es $(, $arg)*) }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_old_windows_option_loads_as_spanish() {
        assert_eq!(serde_json::from_str::<Language>("\"auto\"").unwrap(), Language::Spanish);
        assert_eq!(Language::default(), Language::Spanish);
    }

    #[test]
    fn spanish_is_the_default_until_a_language_is_set() {
        assert_eq!(tr("Hola", "Hello"), "Hola");
        let n = 3;
        assert_eq!(trf!("{n} dispositivos", "{n} devices"), "3 dispositivos");
    }
}
