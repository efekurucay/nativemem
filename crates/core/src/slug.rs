/// Lowercase ASCII slug; runs of other characters collapse to one `-`. Falls back to `item`.
pub fn slugify(input: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for c in input.chars().flat_map(transliterate) {
        if c.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(c.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    if out.is_empty() {
        "item".to_string()
    } else {
        out
    }
}

fn transliterate(c: char) -> Vec<char> {
    let s = match c {
        'ç' | 'Ç' => "c",
        'ğ' | 'Ğ' => "g",
        'ı' | 'İ' => "i",
        'ö' | 'Ö' | 'ø' | 'Ø' => "o",
        'ş' | 'Ş' => "s",
        'ü' | 'Ü' => "u",
        'à'..='å' | 'À'..='Å' => "a",
        'è'..='ë' | 'È'..='Ë' => "e",
        'ì'..='ï' | 'Ì'..='Ï' => "i",
        'ñ' | 'Ñ' => "n",
        'ò'..='ö' | 'Ò'..='Ö' => "o",
        'ù'..='ü' | 'Ù'..='Ü' => "u",
        'ý' | 'ÿ' | 'Ý' => "y",
        'ß' => "ss",
        _ => return vec![c],
    };
    s.chars().collect()
}

#[cfg(test)]
mod tests {
    use super::slugify;

    #[test]
    fn basic() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("  a   b__c  "), "a-b-c");
        assert_eq!(slugify("Already-slug"), "already-slug");
    }

    #[test]
    fn non_ascii() {
        assert_eq!(slugify("Çağrı Şükrü"), "cagri-sukru");
        assert_eq!(slugify("日本語"), "item");
    }

    #[test]
    fn fallback() {
        assert_eq!(slugify(""), "item");
        assert_eq!(slugify("---"), "item");
    }
}
