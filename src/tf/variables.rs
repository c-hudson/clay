//! Variable storage and substitution for TinyFugue compatibility.

use super::TfEngine;

/// Perform variable substitution on text.
///
/// Supports:
/// - `%{varname}` - Standard TF variable substitution
/// - `%varname` - Short form (ends at non-alphanumeric)
/// - `%%` - Literal percent sign
pub fn substitute_variables(engine: &TfEngine, text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '%' {
            if i + 1 < len {
                match chars[i + 1] {
                    // %% -> literal %
                    '%' => {
                        result.push('%');
                        i += 2;
                    }
                    // %{varname} form
                    '{' => {
                        if let Some((var_name, end_idx)) = extract_braced_var(&chars, i + 2) {
                            if let Some(value) = engine.get_var(&var_name) {
                                result.push_str(&value.to_string_value());
                            }
                            // If variable not found, substitute empty string (TF behavior)
                            i = end_idx + 1;
                        } else {
                            // Malformed, keep as-is
                            result.push('%');
                            i += 1;
                        }
                    }
                    // %varname form - variable name is alphanumeric + underscore
                    c if c.is_alphabetic() || c == '_' => {
                        let (var_name, end_idx) = extract_simple_var(&chars, i + 1);
                        if let Some(value) = engine.get_var(&var_name) {
                            result.push_str(&value.to_string_value());
                        }
                        // If variable not found, substitute empty string
                        i = end_idx;
                    }
                    // %n (digit) - positional parameter (handled separately in macro execution)
                    c if c.is_ascii_digit() => {
                        // For now, keep as-is; will be handled in macro context
                        result.push('%');
                        result.push(c);
                        i += 2;
                    }
                    // %P forms for capture groups
                    'P' => {
                        // Keep as-is for now; handled in trigger context
                        result.push('%');
                        i += 1;
                    }
                    // Unknown, keep literal
                    _ => {
                        result.push('%');
                        i += 1;
                    }
                }
            } else {
                // Trailing %, keep as-is
                result.push('%');
                i += 1;
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Extract variable name from %{varname} form.
/// Returns (variable_name, index_of_closing_brace) or None if malformed.
fn extract_braced_var(chars: &[char], start: usize) -> Option<(String, usize)> {
    let mut name = String::new();
    let mut i = start;

    while i < chars.len() {
        match chars[i] {
            '}' => return Some((name, i)),
            c => {
                name.push(c);
                i += 1;
            }
        }
    }

    None  // No closing brace found
}

/// Extract variable name from %varname form.
/// Returns (variable_name, index_after_last_char).
fn extract_simple_var(chars: &[char], start: usize) -> (String, usize) {
    let mut name = String::new();
    let mut i = start;

    while i < chars.len() {
        let c = chars[i];
        if c.is_alphanumeric() || c == '_' {
            name.push(c);
            i += 1;
        } else {
            break;
        }
    }

    (name, i)
}

/// Substitute positional parameters (%1-%9, %*, %L, %R) in macro body.
/// Used when executing macros/actions with arguments.
pub fn substitute_positional(text: &str, args: &[&str]) -> String {
    let mut result = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '%' && i + 1 < len {
            match chars[i + 1] {
                // %1-%9 positional parameters
                c @ '1'..='9' => {
                    let idx = (c as usize) - ('1' as usize);
                    if idx < args.len() {
                        result.push_str(args[idx]);
                    }
                    i += 2;
                }
                // %0 is the macro name (not typically used in body)
                '0' => {
                    i += 2;
                }
                // %* all arguments
                '*' => {
                    result.push_str(&args.join(" "));
                    i += 2;
                }
                // %# number of arguments
                '#' => {
                    result.push_str(&args.len().to_string());
                    i += 2;
                }
                // %P forms for regex capture groups
                'P' if i + 2 < len => {
                    match chars[i + 2] {
                        // %Pn positional capture
                        c @ '0'..='9' => {
                            // Will be handled with captures parameter
                            result.push('%');
                            result.push('P');
                            result.push(c);
                            i += 3;
                        }
                        // %PL left of match
                        'L' => {
                            result.push_str("%PL");
                            i += 3;
                        }
                        // %PR right of match
                        'R' => {
                            result.push_str("%PR");
                            i += 3;
                        }
                        // %P* all captures
                        '*' => {
                            result.push_str("%P*");
                            i += 3;
                        }
                        _ => {
                            result.push('%');
                            i += 1;
                        }
                    }
                }
                _ => {
                    result.push('%');
                    i += 1;
                }
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Substitute regex capture groups in text.
/// %P0 is full match, %P1-%P9 are capture groups.
pub fn substitute_captures(text: &str, full_match: &str, captures: &[&str], left: &str, right: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '%' && i + 1 < len && chars[i + 1] == 'P' && i + 2 < len {
            match chars[i + 2] {
                '0' => {
                    result.push_str(full_match);
                    i += 3;
                }
                c @ '1'..='9' => {
                    let idx = (c as usize) - ('1' as usize);
                    if idx < captures.len() {
                        result.push_str(captures[idx]);
                    }
                    i += 3;
                }
                'L' => {
                    result.push_str(left);
                    i += 3;
                }
                'R' => {
                    result.push_str(right);
                    i += 3;
                }
                '*' => {
                    result.push_str(&captures.join(" "));
                    i += 3;
                }
                _ => {
                    result.push(chars[i]);
                    i += 1;
                }
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tf::TfValue;

    #[test]
    fn test_substitute_braced_var() {
        let mut engine = TfEngine::new();
        engine.set_global("foo", TfValue::String("bar".to_string()));
        engine.set_global("num", TfValue::Integer(42));

        assert_eq!(substitute_variables(&engine, "hello %{foo} world"), "hello bar world");
        assert_eq!(substitute_variables(&engine, "value is %{num}"), "value is 42");
        assert_eq!(substitute_variables(&engine, "%{foo}%{num}"), "bar42");
        assert_eq!(substitute_variables(&engine, "%{undefined}"), "");
    }

    #[test]
    fn test_substitute_simple_var() {
        let mut engine = TfEngine::new();
        engine.set_global("foo", TfValue::String("bar".to_string()));
        engine.set_global("x", TfValue::Integer(5));

        assert_eq!(substitute_variables(&engine, "hello %foo world"), "hello bar world");
        assert_eq!(substitute_variables(&engine, "%x + %x = 10"), "5 + 5 = 10");
        assert_eq!(substitute_variables(&engine, "%foo.txt"), "bar.txt");
    }

    #[test]
    fn test_substitute_percent_escape() {
        let engine = TfEngine::new();
        assert_eq!(substitute_variables(&engine, "100%%"), "100%");
        assert_eq!(substitute_variables(&engine, "%%%%"), "%%");
    }

    #[test]
    fn test_substitute_positional() {
        let args = vec!["one", "two", "three"];
        assert_eq!(substitute_positional("arg1=%1 arg2=%2", &args), "arg1=one arg2=two");
        assert_eq!(substitute_positional("all=%*", &args), "all=one two three");
        assert_eq!(substitute_positional("count=%#", &args), "count=3");
        assert_eq!(substitute_positional("%9 is empty", &args), " is empty");
    }

    #[test]
    fn test_substitute_captures() {
        let captures = vec!["group1", "group2"];
        assert_eq!(
            substitute_captures("matched %P0, first=%P1", "fullmatch", &captures, "left", "right"),
            "matched fullmatch, first=group1"
        );
        assert_eq!(
            substitute_captures("%PL[%P0]%PR", "MATCH", &captures, "before ", " after"),
            "before [MATCH] after"
        );
    }
}
