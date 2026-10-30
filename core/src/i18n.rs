//! Idioma de la interfaz. Cada texto va en el sitio donde se usa, en los dos idiomas (`tr` y `trf!`):
//! así ninguno se queda sin traducir y el compilador comprueba los datos de cada formato. Los logs
//! no se traducen.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;

/// Idioma primario "español" de Windows.
const LANG_SPANISH: u16 = 0x0a;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    /// Español si Windows está en español; si no, inglés.
    #[default]
    Auto,
    Spanish,
    English,
}

static ENGLISH: AtomicBool = AtomicBool::new(false);

/// Aplica el idioma a todo el proceso.
pub fn set(language: Language) {
    let english = match language {
        Language::Spanish => false,
        Language::English => true,
        Language::Auto => unsafe { GetUserDefaultUILanguage() & 0x3ff != LANG_SPANISH },
    };
    ENGLISH.store(english, Ordering::Relaxed);
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
    fn spanish_is_the_default_until_a_language_is_set() {
        assert_eq!(tr("Hola", "Hello"), "Hola");
        let n = 3;
        assert_eq!(trf!("{n} dispositivos", "{n} devices"), "3 dispositivos");
    }
}
