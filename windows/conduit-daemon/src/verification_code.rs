//! Verification-code extraction for mirrored phone notifications.
//!
//! Deliberately context-gated: a four-to-eight digit run is not a code merely because it exists.
//! We require a nearby verification keyword and never log or persist the extracted value.

use crate::wire::pb;

const MAX_KEYWORD_DISTANCE: usize = 80;
const KEYWORDS: &[&str] = &[
    "verification code",
    "security code",
    "authentication code",
    "login code",
    "one-time code",
    "one time code",
    "one-time password",
    "one time password",
    "passcode",
    "your code",
    "otp",
    "验证码",
    "校验码",
    "动态码",
    "认证码",
    "短信码",
    "安全码",
    "一次性密码",
    "口令",
    "認証コード",
    "確認コード",
    "인증번호",
    "인증 코드",
    "code de vérification",
    "code de sécurité",
    "bestätigungscode",
    "sicherheitscode",
    "código de verificación",
    "código de seguridad",
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

fn extract_from_text(text: &str) -> Option<String> {
    let text = text.to_lowercase();
    let keywords = KEYWORDS
        .iter()
        .flat_map(|keyword| {
            text.match_indices(keyword)
                .map(move |(start, _)| (start, start + keyword.len()))
        })
        .collect::<Vec<_>>();
    if keywords.is_empty() {
        return None;
    }

    digit_candidates(&text)
        .into_iter()
        .filter_map(|candidate| {
            let distance = keywords
                .iter()
                .map(|&(start, end)| span_distance(candidate.start, candidate.end, start, end))
                .min()?;
            (distance <= MAX_KEYWORD_DISTANCE).then_some((distance, candidate))
        })
        .min_by_key(|(distance, candidate)| (*distance, candidate.code.len()))
        .map(|(_, candidate)| candidate.code)
}

fn digit_candidates(text: &str) -> Vec<Candidate> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() || (i > 0 && bytes[i - 1].is_ascii_digit()) {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i;
        let mut code = String::with_capacity(8);
        while j < bytes.len() {
            if bytes[j].is_ascii_digit() {
                code.push(bytes[j] as char);
                j += 1;
                continue;
            }
            if bytes[j] == b'-' && j + 1 < bytes.len() && bytes[j + 1].is_ascii_digit() {
                j += 1;
                continue;
            }
            break;
        }
        if (4..=8).contains(&code.len()) {
            out.push(Candidate { code, start, end: j });
        }
        i = j.max(i + 1);
    }
    out
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
        assert_eq!(extract_from_text("【银行】验证码 482731，5分钟内有效"), Some("482731".into()));
        assert_eq!(extract_from_text("Your verification code is 130944"), Some("130944".into()));
    }

    #[test]
    fn normalizes_a_hyphenated_code() {
        assert_eq!(extract_from_text("Your security code is 123-456"), Some("123456".into()));
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
    fn unrelated_numbers_and_long_phone_numbers_are_rejected() {
        assert_eq!(extract_from_text("Order 482731 shipped"), None);
        assert_eq!(extract_from_text("Your verification code help line is 18001234567"), None);
        assert_eq!(extract_from_text("2026-08-31 monthly statement"), None);
    }

    #[test]
    fn closest_candidate_to_keyword_wins() {
        assert_eq!(
            extract_from_text("reference 777777 — your verification code is 246810"),
            Some("246810".into()),
        );
    }
}
