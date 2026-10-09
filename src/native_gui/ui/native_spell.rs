//! The system's own spell checker, where there is one: Windows' (ISpellChecker, in the
//! user's language) and macOS's (NSSpellChecker), as the WebView's text field uses. On
//! Linux, and wherever the user has set a dictionary file, Clay's own checker
//! (`spell.rs`) does it instead.
//!
//! Both are used from the window's thread only (COM's apartment, AppKit's main thread).

/// A system spell checker.
pub enum Native {
    #[cfg(windows)]
    Windows(win::Checker),
    #[cfg(target_os = "macos")]
    Mac,
}

impl Native {
    /// The system's checker, if it has one that works.
    pub fn new() -> Option<Native> {
        #[cfg(windows)]
        {
            win::Checker::new().map(Native::Windows)
        }
        #[cfg(target_os = "macos")]
        {
            Some(Native::Mac)
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            None
        }
    }

    pub fn is_valid(&self, word: &str) -> bool {
        match self {
            #[cfg(windows)]
            Native::Windows(c) => c.is_valid(word),
            #[cfg(target_os = "macos")]
            Native::Mac => mac::is_valid(word),
            #[cfg(not(any(windows, target_os = "macos")))]
            _ => {
                let _ = word;
                true
            }
        }
    }

    pub fn suggestions(&self, word: &str, count: usize) -> Vec<String> {
        match self {
            #[cfg(windows)]
            Native::Windows(c) => c.suggestions(word, count),
            #[cfg(target_os = "macos")]
            Native::Mac => mac::suggestions(word, count),
            #[cfg(not(any(windows, target_os = "macos")))]
            _ => {
                let _ = (word, count);
                Vec::new()
            }
        }
    }
}

#[cfg(windows)]
mod win {
    use windows::Win32::Globalization::{GetUserDefaultLocaleName, ISpellChecker, ISpellCheckerFactory, SpellCheckerFactory};
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree};
    use windows::core::{HSTRING, PWSTR};

    pub struct Checker {
        checker: ISpellChecker,
    }

    impl Checker {
        pub fn new() -> Option<Checker> {
            unsafe {
                // GPUI has COM up on this thread already; this only makes sure.
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                let factory: ISpellCheckerFactory = CoCreateInstance(&SpellCheckerFactory, None, CLSCTX_INPROC_SERVER).ok()?;
                let mut buf = [0u16; 85];
                let n = GetUserDefaultLocaleName(&mut buf);
                let user = String::from_utf16_lossy(&buf[..(n.max(1) as usize - 1).min(buf.len())]);
                let lang = [user.as_str(), "en-US"]
                    .into_iter()
                    .find(|l| !l.is_empty() && factory.IsSupported(&HSTRING::from(*l)).is_ok_and(|b| b.as_bool()))?
                    .to_string();
                let checker = factory.CreateSpellChecker(&HSTRING::from(lang)).ok()?;
                Some(Checker { checker })
            }
        }

        pub fn is_valid(&self, word: &str) -> bool {
            unsafe {
                let Ok(errors) = self.checker.Check(&HSTRING::from(word)) else { return true };
                let mut first = None;
                // S_OK with an error, S_FALSE when there are none.
                errors.Next(&mut first).0 != 0 || first.is_none()
            }
        }

        pub fn suggestions(&self, word: &str, count: usize) -> Vec<String> {
            let mut out = Vec::new();
            unsafe {
                let Ok(list) = self.checker.Suggest(&HSTRING::from(word)) else { return out };
                while out.len() < count {
                    let mut item = [PWSTR::null()];
                    let mut fetched = 0u32;
                    if list.Next(&mut item, Some(&mut fetched)).0 != 0 || fetched == 0 {
                        break;
                    }
                    if let Ok(s) = item[0].to_string() {
                        out.push(s);
                    }
                    CoTaskMemFree(Some(item[0].0 as *const _));
                }
            }
            out
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2_app_kit::NSSpellChecker;
    use objc2_foundation::{NSRange, NSString};

    pub fn is_valid(word: &str) -> bool {
        let checker = NSSpellChecker::sharedSpellChecker();
        let range = checker.checkSpellingOfString_startingAt(&NSString::from_str(word), 0);
        range.length == 0
    }

    pub fn suggestions(word: &str, count: usize) -> Vec<String> {
        let checker = NSSpellChecker::sharedSpellChecker();
        let s = NSString::from_str(word);
        let range = NSRange::new(0, s.length());
        let Some(guesses) = checker.guessesForWordRange_inString_language_inSpellDocumentWithTag(range, &s, None, 0) else {
            return Vec::new();
        };
        (0..guesses.count().min(count)).map(|i| guesses.objectAtIndex(i).to_string()).collect()
    }
}

#[cfg(test)]
mod tests {
    /// On a system with its own checker, it knows English.
    #[test]
    fn the_system_checker_knows_common_words() {
        let Some(n) = super::Native::new() else { return };
        assert!(n.is_valid("the"));
        assert!(!n.is_valid("qwzxv"));
        assert!(n.suggestions("teh", 5).iter().any(|s| s == "the"), "{:?}", n.suggestions("teh", 5));
    }
}
