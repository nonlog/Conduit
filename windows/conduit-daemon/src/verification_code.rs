//! Verification-code (OTP) extraction for mirrored phone notifications.
//!
//! Deliberately context-gated: extracts 4–8 digit codes (including prefixed forms
//! like `G-123456` / `FB-12345` and grouped forms like `123-456` / `1234-5678`)
//! when they appear near an OTP keyword in English, Chinese, or other common
//! languages, while rejecting dates, times, currency amounts, phone numbers,
//! promo/zip/error codes, and order/card/account numbers.
//! Extracted values are never logged or persisted.

use crate::wire::pb;

const MAX_OTP_TEXT_BYTES: usize = 1024;
const MAX_KEYWORD_DISTANCE: usize = 100;

/// Word-bounded keywords (matched as whole words in lowercased characters).
const WORD_KEYWORDS: &[&str] = &[
    // English
    "otp",
    "code",
    "codes",
    "passcode",
    "password",
    "verification",
    "verify",
    "pin",
    "login",
    "signin",
    "2fa",
    "mfa",
    "authentication",
    "authenticator",
    // Spanish / Portuguese / Italian / French
    "código",
    "codigo",
    "verificación",
    "verificacion",
    "verificação",
    "verificacao",
    "acceso",
    "acesso",
    "clave",
    "senha",
    "codice",
    "verifica",
    "accesso",
    "vérification",
    "connexion",
    "authentification",
    // German / Dutch / Turkish
    "bestätigungscode",
    "bestatigungscode",
    "bestätigung",
    "einmalpasswort",
    "sicherheitscode",
    "anmeldung",
    "verifizierung",
    "zugangscode",
    "prüfcode",
    "verificatiecode",
    "beveiligingscode",
    "inlogcode",
    "doğrulama",
    "dogrulama",
    // Russian / Ukrainian / Arabic / Hindi
    "код",
    "подтверждения",
    "проверочный",
    "одноразовый",
    "пароль",
    "підтвердження",
    "رمز",
    "تحقق",
    "التحقق",
    "كود",
    "تأكيد",
    "कोड",
    "ओटीपी",
    "सत्यापन",
    "पासकोड",
    "पिन",
];

/// Multi-word phrases and CJK substrings.
const PHRASE_KEYWORDS: &[&str] = &[
    "one-time",
    "one time",
    "sign in",
    "sign-in",
    "log in",
    "log-in",
    "two-factor",
    "two factor",
    "security code",
    "verification code",
    "authentication code",
    "confirmation code",
    "login code",
    "access code",
    "temporary password",
    "iniciar sesión",
    "iniciar sesion",
    "mot de passe",
    "einmal-passwort",
    "doğrulama kodu",
    "dogrulama kodu",
    "onay kodu",
    "güvenlik kodu",
    // Chinese (Simplified & Traditional)
    "验证码",
    "驗證碼",
    "校验码",
    "校驗碼",
    "动态码",
    "動態碼",
    "认证码",
    "認證碼",
    "短信码",
    "安全码",
    "安全碼",
    "登录码",
    "登錄碼",
    "确认码",
    "確認碼",
    "一次性密码",
    "一次性密碼",
    "口令",
    // Japanese
    "認証コード",
    "確認コード",
    "ワンタイムパスワード",
    "セキュリティコード",
    "暗証番号",
    "認証番号",
    // Korean
    "인증번호",
    "인증 번호",
    "확인코드",
    "확인 코드",
    "보안코드",
    "보안 코드",
    "일회용 비밀번호",
    "핀번호",
];

/// Words immediately preceding `"code"` / `"codes"` that indicate a non-OTP code.
const NON_OTP_CODE_PREFIXES: &[&str] = &[
    "zip",
    "postal",
    "area",
    "country",
    "promo",
    "promotion",
    "promotional",
    "discount",
    "coupon",
    "voucher",
    "referral",
    "invite",
    "bar",
    "qr",
    "dress",
    "source",
    "error",
    "status",
    "fault",
    "tracking",
    "sort",
    "swift",
    "bic",
    "ifsc",
    "hs",
    "color",
    "colour",
    "product",
    "item",
    "tax",
];

#[derive(Debug)]
struct Candidate {
    code: String,
    start: usize,
    end: usize,
}

pub fn extract(title: &str, body: &str, messages: &[pb::TextMessage]) -> Option<String> {
    let mut text = String::with_capacity(title.len() + body.len() + messages.len() * 32 + 2);
    text.push_str(title);
    text.push('\n');
    text.push_str(body);
    for message in messages {
        text.push('\n');
        if !message.sender.is_empty() {
            text.push_str(&message.sender);
            text.push_str(": ");
        }
        text.push_str(&message.text);
    }
    extract_from_text(&text)
}

pub fn extract_from_text(body: &str) -> Option<String> {
    if body.is_empty() || body.len() > MAX_OTP_TEXT_BYTES {
        return None;
    }
    let chars: Vec<char> = body.chars().collect();
    // One lowercase char per original char so indices stay aligned even for "İ".
    let lower_chars: Vec<char> = chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let keyword_spans = find_keyword_spans(&lower_chars);
    if keyword_spans.is_empty() {
        return None;
    }

    find_candidates(&chars, &lower_chars)
        .into_iter()
        .filter_map(|cand| {
            let dist = keyword_spans
                .iter()
                .map(|&(ks, ke)| span_distance(cand.start, cand.end, ks, ke))
                .min()?;
            (dist <= MAX_KEYWORD_DISTANCE).then_some((dist, cand.code))
        })
        .min_by_key(|(dist, code)| (*dist, code.len()))
        .map(|(_, code)| code)
}

fn find_keyword_spans(lower_chars: &[char]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();

    for &phrase in PHRASE_KEYWORDS {
        let phrase_chars: Vec<char> = phrase.chars().collect();
        let plen = phrase_chars.len();
        if plen == 0 || lower_chars.len() < plen {
            continue;
        }
        let ascii_word = phrase_chars
            .first()
            .is_some_and(char::is_ascii_alphanumeric);
        for i in 0..=(lower_chars.len() - plen) {
            if lower_chars[i..i + plen] == phrase_chars[..] {
                if ascii_word {
                    let before_ok = i == 0 || !lower_chars[i - 1].is_alphanumeric();
                    let after_ok =
                        i + plen == lower_chars.len() || !lower_chars[i + plen].is_alphanumeric();
                    if !before_ok || !after_ok {
                        continue;
                    }
                }
                spans.push((i, i + plen));
            }
        }
    }

    let mut i = 0;
    let mut prev_word: Option<(usize, usize, String)> = None;
    while i < lower_chars.len() {
        if !lower_chars[i].is_alphanumeric() {
            i += 1;
            continue;
        }
        let start = i;
        while i < lower_chars.len() && lower_chars[i].is_alphanumeric() {
            i += 1;
        }
        let word: String = lower_chars[start..i].iter().collect();
        if WORD_KEYWORDS.contains(&word.as_str()) {
            let is_non_otp_code = (word == "code" || word == "codes")
                && prev_word.as_ref().is_some_and(|(_, pe, pw)| {
                    start.saturating_sub(*pe) <= 2 && NON_OTP_CODE_PREFIXES.contains(&pw.as_str())
                });
            if !is_non_otp_code {
                spans.push((start, i));
            }
        }
        prev_word = Some((start, i, word));
    }

    spans
}

fn find_candidates(chars: &[char], lower_chars: &[char]) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        // Prefixed format like "G-123456" or "FB-12345".
        if chars[i].is_ascii_uppercase() {
            let before_ok = i == 0 || !chars[i - 1].is_alphanumeric();
            if before_ok {
                let mut p_end = i;
                while p_end < chars.len() && chars[p_end].is_ascii_uppercase() && p_end - i <= 3 {
                    p_end += 1;
                }
                let p_len = p_end - i;
                if (1..=3).contains(&p_len)
                    && chars.get(p_end) == Some(&'-')
                    && chars.get(p_end + 1).is_some_and(char::is_ascii_digit)
                {
                    let d_start = p_end + 1;
                    let mut d_end = d_start;
                    while d_end < chars.len() && chars[d_end].is_ascii_digit() {
                        d_end += 1;
                    }
                    let d_len = d_end - d_start;
                    if (4..=8).contains(&d_len)
                        && is_valid_after(chars, lower_chars, d_end)
                        && !is_excluded_context(chars, lower_chars, i, d_end)
                    {
                        out.push(Candidate {
                            start: i,
                            end: d_end,
                            code: chars[d_start..d_end].iter().collect(),
                        });
                        i = d_end;
                        continue;
                    }
                }
            }
        }

        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }

        let start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        let len = i - start;

        // Grouped codes like "123-456" or "1234-5678".
        if (len == 3 || len == 4)
            && chars.get(i) == Some(&'-')
            && chars.get(i + 1).is_some_and(char::is_ascii_digit)
        {
            let d2_start = i + 1;
            let mut d2_end = d2_start;
            while d2_end < chars.len() && chars[d2_end].is_ascii_digit() {
                d2_end += 1;
            }
            if d2_end - d2_start == len
                && is_valid_before(chars, start)
                && is_valid_after(chars, lower_chars, d2_end)
                && chars.get(d2_end) != Some(&'-')
                && !is_excluded_context(chars, lower_chars, start, d2_end)
            {
                let mut code: String = chars[start..i].iter().collect();
                code.extend(chars[d2_start..d2_end].iter());
                out.push(Candidate {
                    start,
                    end: d2_end,
                    code,
                });
                i = d2_end;
                continue;
            }
        }

        if (4..=8).contains(&len)
            && is_valid_before(chars, start)
            && is_valid_after(chars, lower_chars, i)
            && !is_excluded_context(chars, lower_chars, start, i)
        {
            out.push(Candidate {
                start,
                end: i,
                code: chars[start..i].iter().collect(),
            });
        }
    }
    out
}

fn is_valid_before(chars: &[char], start: usize) -> bool {
    let Some(prev) = start.checked_sub(1).map(|idx| chars[idx]) else {
        return true;
    };
    if prev.is_alphanumeric() || is_currency_symbol(prev) || matches!(prev, '.' | ',' | '+') {
        return false;
    }
    if matches!(prev, '/' | ':') && start >= 2 && chars[start - 2].is_ascii_digit() {
        return false;
    }
    true
}

fn is_valid_after(chars: &[char], lower_chars: &[char], end: usize) -> bool {
    let Some(&next) = chars.get(end) else {
        return true;
    };
    if next.is_alphanumeric() || is_currency_symbol(next) || next == '%' {
        return false;
    }
    if matches!(next, '.' | ',' | '/' | ':') && chars.get(end + 1).is_some_and(char::is_ascii_digit)
    {
        return false;
    }
    let mut k = end;
    while k < chars.len() && (chars[k] == ' ' || chars[k] == '\t') {
        k += 1;
    }
    if k > end && k < chars.len() {
        if is_currency_symbol(chars[k]) || chars[k] == '%' {
            return false;
        }
        if lower_chars[k].is_alphabetic() {
            let w_start = k;
            let mut w_end = k;
            while w_end < lower_chars.len() && lower_chars[w_end].is_alphabetic() {
                w_end += 1;
            }
            let word: String = lower_chars[w_start..w_end].iter().collect();
            if matches!(
                word.as_str(),
                "usd"
                    | "eur"
                    | "gbp"
                    | "inr"
                    | "jpy"
                    | "cad"
                    | "aud"
                    | "chf"
                    | "cny"
                    | "rmb"
                    | "dollars"
                    | "dollar"
                    | "euros"
                    | "euro"
                    | "pounds"
                    | "pound"
                    | "rupees"
                    | "rupee"
                    | "yen"
                    | "yuan"
                    | "hrs"
                    | "am"
                    | "pm"
            ) {
                return false;
            }
        }
    }
    true
}

fn is_currency_symbol(c: char) -> bool {
    matches!(
        c,
        '₹' | '$'
            | '€'
            | '£'
            | '¥'
            | '￥'
            | '₩'
            | '₽'
            | '₺'
            | '₫'
            | '฿'
            | '₴'
            | '₦'
            | '₱'
            | '₪'
    )
}

fn is_excluded_context(chars: &[char], lower_chars: &[char], start: usize, end: usize) -> bool {
    let len = end - start;

    // 1. Date patterns like YYYY-MM-DD or DD-MM-YYYY.
    if len == 4 {
        if chars.get(end) == Some(&'-') {
            let m_start = end + 1;
            let mut m_end = m_start;
            while m_end < chars.len() && chars[m_end].is_ascii_digit() {
                m_end += 1;
            }
            if (1..=2).contains(&(m_end - m_start)) && chars.get(m_end) == Some(&'-') {
                return true;
            }
        }
        if start >= 2 && chars[start - 1] == '-' {
            let mut d_start = start - 1;
            while d_start > 0 && chars[d_start - 1].is_ascii_digit() {
                d_start -= 1;
            }
            let d_len = (start - 1) - d_start;
            if (1..=2).contains(&d_len) && d_start >= 2 && chars[d_start - 1] == '-' {
                return true;
            }
        }
    }

    // 2. Phone number patterns.
    if start > 0 {
        let mut b = start;
        while b > 0 && matches!(chars[b - 1], ' ' | '-' | ')' | '.') {
            b -= 1;
        }
        let sep_slice = &chars[b..start];
        let has_phone_sep = sep_slice.contains(&'-') || sep_slice.contains(&')');
        let mut d_start = b;
        while d_start > 0 && chars[d_start - 1].is_ascii_digit() {
            d_start -= 1;
        }
        let prev_digits = b - d_start;
        if (2..=5).contains(&prev_digits) {
            if has_phone_sep {
                return true;
            }
            let before_prev = d_start.checked_sub(1).map(|idx| chars[idx]);
            if before_prev == Some('+')
                || before_prev == Some('(')
                || chars.get(d_start) == Some(&'0')
            {
                return true;
            }
        }
    }
    {
        let mut b = start;
        while b > 0 && chars[b - 1] == ' ' {
            b -= 1;
        }
        if b > 0 && chars[b - 1] == '+' {
            return true;
        }
    }

    // 3. Currency symbol or prefix with space/dot before the number.
    {
        let mut b = start;
        while b > 0 && matches!(chars[b - 1], ' ' | '\t' | '.') {
            b -= 1;
        }
        if b > 0 && is_currency_symbol(chars[b - 1]) {
            return true;
        }
        let mut w_start = b;
        while w_start > 0 && lower_chars[w_start - 1].is_alphabetic() {
            w_start -= 1;
        }
        if w_start < b && (w_start == 0 || !lower_chars[w_start - 1].is_alphanumeric()) {
            let word: String = lower_chars[w_start..b].iter().collect();
            if matches!(
                word.as_str(),
                "rs" | "inr"
                    | "usd"
                    | "eur"
                    | "gbp"
                    | "jpy"
                    | "cad"
                    | "aud"
                    | "chf"
                    | "cny"
                    | "rmb"
                    | "brl"
                    | "mxn"
            ) {
                return true;
            }
        }
    }

    // 4. Preceding '#' (e.g. "order #123456"), unless preceded by an OTP word.
    if start > 0 && chars[start - 1] == '#' {
        let mut b = start - 1;
        while b > 0 && chars[b - 1] == ' ' {
            b -= 1;
        }
        let mut w_start = b;
        while w_start > 0 && lower_chars[w_start - 1].is_alphanumeric() {
            w_start -= 1;
        }
        let word: String = lower_chars[w_start..b].iter().collect();
        if !matches!(word.as_str(), "code" | "otp" | "pin" | "passcode") {
            return true;
        }
    }

    // 5. Non-OTP words immediately preceding the number within the same clause.
    let prev_words = preceding_clause_words(lower_chars, start, 3);
    for (idx, w) in prev_words.iter().enumerate() {
        let is_generic_code = matches!(
            w.as_str(),
            "code" | "código" | "codigo" | "codice" | "kod" | "kodu" | "код" | "कोड"
        );
        if is_generic_code {
            if prev_words
                .get(idx + 1)
                .is_some_and(|prefix| NON_OTP_CODE_PREFIXES.contains(&prefix.as_str()))
            {
                return true;
            }
            break;
        }
        if WORD_KEYWORDS.contains(&w.as_str()) {
            break;
        }
        if matches!(
            w.as_str(),
            "order"
                | "pedido"
                | "commande"
                | "bestellung"
                | "ordine"
                | "tracking"
                | "shipment"
                | "delivery"
                | "invoice"
                | "receipt"
                | "transaction"
                | "txn"
                | "ref"
                | "reference"
                | "account"
                | "acct"
                | "card"
                | "ending"
                | "terminada"
                | "ticket"
                | "item"
                | "sku"
                | "ext"
                | "extension"
                | "call"
                | "dial"
                | "phone"
                | "tel"
        ) {
            return true;
        }
        if len == 4
            && matches!(
                w.as_str(),
                "jan"
                    | "january"
                    | "feb"
                    | "february"
                    | "mar"
                    | "march"
                    | "apr"
                    | "april"
                    | "may"
                    | "jun"
                    | "june"
                    | "jul"
                    | "july"
                    | "aug"
                    | "august"
                    | "sep"
                    | "sept"
                    | "september"
                    | "oct"
                    | "october"
                    | "nov"
                    | "november"
                    | "dec"
                    | "december"
            )
        {
            return true;
        }
    }

    false
}

fn preceding_clause_words(lower_chars: &[char], pos: usize, max_words: usize) -> Vec<String> {
    let mut words = Vec::new();
    let mut i = pos;
    let limit = pos.saturating_sub(32);
    while i > limit && words.len() < max_words {
        while i > limit && !lower_chars[i - 1].is_alphanumeric() {
            if matches!(lower_chars[i - 1], '.' | '!' | '?' | ';' | '\n' | '\r') {
                return words;
            }
            i -= 1;
        }
        if i <= limit {
            break;
        }
        let w_end = i;
        while i > 0 && lower_chars[i - 1].is_alphanumeric() {
            i -= 1;
        }
        words.push(lower_chars[i..w_end].iter().collect());
    }
    words
}

fn span_distance(a_start: usize, a_end: usize, b_start: usize, b_end: usize) -> usize {
    if a_end <= b_start {
        b_start - a_end
    } else if b_end <= a_start {
        a_start - b_end
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_chinese_and_english_codes() {
        assert_eq!(
            extract_from_text("【银行】验证码 482731，5分钟内有效"),
            Some("482731".into())
        );
        assert_eq!(
            extract_from_text("Your verification code is 130944"),
            Some("130944".into())
        );
        assert_eq!(
            extract_from_text("Use 9182 to verify your account."),
            Some("9182".into())
        );
        assert_eq!(
            extract_from_text("Your 2FA code for GitHub is 394821."),
            Some("394821".into())
        );
    }

    #[test]
    fn detects_prefixed_and_hyphenated_codes() {
        assert_eq!(
            extract_from_text("G-482193 is your Google verification code."),
            Some("482193".into())
        );
        assert_eq!(
            extract_from_text("FB-84920 is your Facebook confirmation code"),
            Some("84920".into())
        );
        assert_eq!(
            extract_from_text("Your security code is 123-456"),
            Some("123456".into())
        );
        assert_eq!(
            extract_from_text("Use code 1234-5678"),
            Some("12345678".into())
        );
    }

    #[test]
    fn messaging_style_text_is_scanned() {
        let messages = vec![pb::TextMessage {
            sender: "Service".into(),
            text: "Login code: 825104".into(),
        }];
        assert_eq!(extract("Messages", "", &messages), Some("825104".into()));
    }

    #[test]
    fn disambiguates_from_dates_currency_phone_and_order_numbers() {
        assert_eq!(
            extract_from_text("On 2026-10-08, your verification code is 482913."),
            Some("482913".into())
        );
        assert_eq!(
            extract_from_text("Order #849201: use verification code 591024 at pickup."),
            Some("591024".into())
        );
        assert_eq!(
            extract_from_text("Payment of $1200 to Acct 9988: enter passcode 445566 to authorize."),
            Some("445566".into())
        );
        assert_eq!(
            extract_from_text("Call 555-0100 if you didn't request verification code 314159."),
            Some("314159".into())
        );
    }

    #[test]
    fn rejects_non_otp_codes_amounts_dates_and_phones() {
        for sample in [
            "Order 482731 shipped",
            "Your verification code help line is 18001234567",
            "2026-08-31 monthly statement",
            "Enter promo code 2026 at checkout!",
            "Delivery to zip code 94043 is scheduled.",
            "Login failed with error code 4040.",
            "Use your PIN at checkout to pay $1250.",
            "Verification complete: €4500 transferred.",
            "Your login was recorded on 2026-10-08 at 12:30.",
            "To reset your password, call 555-0100 or +1 800-555-0199.",
            "Login alert for card ending 4821.",
        ] {
            assert_eq!(extract_from_text(sample), None, "should reject: {sample}");
        }
    }

    #[test]
    fn closest_candidate_to_keyword_wins() {
        assert_eq!(
            extract_from_text("reference 777777 — your verification code is 246810"),
            Some("246810".into()),
        );
    }
}
