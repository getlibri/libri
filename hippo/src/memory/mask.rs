//! Secrets kept out of the log, best effort: every message is masked
//! before it is written, against the known shapes below; a secret of any
//! other shape gets through. The raw transcript archive (root-owned) keeps
//! the originals.

use regex::Regex;
use std::sync::LazyLock;

pub const MASK: &str = "[secret masked]";

/// A prefixed key's value holds an unbroken alphanumeric stretch this long.
/// Tool, session and message ids run shorter, up to 32.
const PREFIXED_RUN: usize = 40;
/// A prefixed value in hex is a key from this length on, and only behind an
/// underscore: `rs_` and `msg_` ids carry 48 hex digits, and a hyphen joins
/// a name to a hash, as in `docker-<hash>.scope`.
const PREFIXED_HEX_RUN: usize = 64;
/// A named value holds an unbroken alphanumeric stretch this long, which
/// spares identifiers and UUIDs. In hex it needs more of a name than plain
/// `key`: `cache key: <hash>` is a hash, `nzb_key = <hex>` is a key.
const NAMED_RUN: usize = 20;

/// The whole pattern list. A capture group named `keep` survives the mask
/// (a header name, say), the rest of the match is replaced. A group named
/// `prefixed` or `named` holds a value the matching check must accept, or
/// the match stands unmasked.
const PATTERNS: &[&str] = &[
    // PEM private keys, whole block.
    r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z0-9 ]*PRIVATE KEY-----",
    // Anthropic, then OpenAI (sk-, sk-proj-, sk-svcacct-...).
    r"\bsk-ant-[A-Za-z0-9_\-]{20,}",
    r"\bsk-[A-Za-z0-9_\-]{20,}",
    // GitHub tokens: classic and fine-grained.
    r"\bgh[pousr]_[A-Za-z0-9]{30,}",
    r"\bgithub_pat_[A-Za-z0-9_]{20,}",
    // Tailscale auth and API keys.
    r"\btskey-[A-Za-z0-9]+-[A-Za-z0-9\-]{8,}",
    // age secret keys.
    r"\bAGE-SECRET-KEY-1[0-9A-Z]{50,}",
    // AWS access key ids and secret keys given by name.
    r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b",
    r"(?i)(?P<keep>aws_secret_access_key\s*[=:]\s*)[A-Za-z0-9/+=]{30,}",
    // Bearer tokens in headers.
    r"(?i)(?P<keep>\bbearer\s+)[A-Za-z0-9._~+/=\-]{16,}",
    // JSON web tokens, wherever they appear.
    r"\beyJ[\w-]{10,}\.eyJ[\w-]{10,}(?:\.[\w-]*)?",
    // Any long value given as a password, secret or token.
    r#"(?i)(?P<keep>(?:password|passwd|secret|token)["']?\s*[=:]\s*)\S{8,}"#,
    // A watch's push address, whose last part is its token.
    r"(?P<keep>/hook/\d+/)[0-9a-f]{64}",
    // A value given under any name ending in key, as env, JSON, YAML or a
    // URL query. A bare colon is left out: `nodekey:<hex>` is a public id.
    r#"(?i)(?P<keep>(?P<name>[a-z0-9_\-]*key)(?:["']?\s*=|["']\s*:|:\s)\s*["']?)(?P<named>[A-Za-z0-9][A-Za-z0-9+/=_\-]{19,})"#,
    // A short lowercase prefix joined to a long random value, whoever issued
    // it. Not after a slash: that is a path or URL, like a Stripe invoice's.
    r"(?m)(?P<keep>^|[^A-Za-z0-9_\-./])[a-z]{2,8}(?P<joiner>[_-])(?P<prefixed>[A-Za-z0-9_\-]{40,})",
];

static RES: LazyLock<Vec<Regex>> =
    LazyLock::new(|| PATTERNS.iter().map(|p| Regex::new(p).unwrap()).collect());

/// The value's longest unbroken alphanumeric stretch.
fn longest_run(value: &str) -> &str {
    value
        .split(|c: char| !c.is_ascii_alphanumeric())
        .max_by_key(|run| run.len())
        .unwrap_or("")
}

fn has_letter_and_digit(run: &str) -> bool {
    run.bytes().any(|b| b.is_ascii_alphabetic()) && run.bytes().any(|b| b.is_ascii_digit())
}

fn is_hex(run: &str) -> bool {
    run.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_prefixed_key(joiner: &str, value: &str) -> bool {
    let run = longest_run(value);
    if run.len() < PREFIXED_RUN || !has_letter_and_digit(run) {
        return false;
    }
    !is_hex(run) || (joiner == "_" && run.len() >= PREFIXED_HEX_RUN)
}

fn is_named_key(name: &str, value: &str) -> bool {
    let run = longest_run(value);
    run.len() >= NAMED_RUN
        && has_letter_and_digit(run)
        && !name.to_ascii_lowercase().contains("public")
        && (name.len() > "key".len() || !is_hex(run))
}

fn is_key(c: &regex::Captures) -> bool {
    if let Some(value) = c.name("prefixed") {
        return is_prefixed_key(&c["joiner"], value.as_str());
    }
    if let Some(value) = c.name("named") {
        return is_named_key(&c["name"], value.as_str());
    }
    true
}

pub fn mask(text: &str) -> String {
    let mut out = text.to_owned();
    for re in RES.iter() {
        if re.is_match(&out) {
            out = re
                .replace_all(&out, |c: &regex::Captures| {
                    if !is_key(c) {
                        return c[0].to_owned();
                    }
                    let keep = c.name("keep").map_or("", |m| m.as_str());
                    format!("{keep}{MASK}")
                })
                .into_owned();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn masked(s: &str) -> bool {
        mask(s).contains(MASK)
    }

    #[test]
    fn masks_each_kind() {
        // Assembled at runtime: whole key-shaped literals in the source
        // set off secret scanners.
        let cases = [
            concat!("key sk-", "ant-api03-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789"),
            concat!("OPENAI_API_KEY=sk-", "proj-AbCdEfGhIjKlMnOpQrStUvWx"),
            concat!("token gh", "p_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789"),
            concat!(
                "github",
                "_pat_11ABCDEFG0123456789_abcdefghijklmnopqrstuvwxyz"
            ),
            concat!("tskey", "-auth-kAbCdEf1CNTRL-AbCdEfGhIjKlMnOpQrStUv"),
            concat!(
                "AGE-SECRET",
                "-KEY-1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ"
            ),
            concat!("AKIA", "IOSFODNN7EXAMPLE"),
            concat!(
                "aws_secret_access_key = wJalrXUtnFEMI/K7MDENG",
                "/bPxRfiCYEXAMPLEKEY"
            ),
            concat!(
                "Authorization: Bearer ",
                "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.abc"
            ),
            concat!(
                "cookie eyJhbGciOiJIUzI1NiJ9",
                ".eyJzdWIiOiIxMjM0NTY3ODkwIn0.sig"
            ),
            concat!("DB_PASSWORD=", "hunter2hunter2"),
            concat!(r#"{"refresh_token": ""#, "AbCdEfGh12345678\"}"),
            concat!(
                "POST /hook/7/0123456789abcdef0123456789abcdef",
                "0123456789abcdef0123456789abcdef on the site"
            ),
            concat!("Signing secret: ", "0123456789abcdef"),
            concat!(
                "-----BEGIN OPENSSH PRIVATE",
                " KEY-----\nb3BlbnNzaC1rZXktdjEA\n-----END OPENSSH PRIVATE",
                " KEY-----"
            ),
        ];
        for case in cases {
            assert!(masked(case), "not masked: {case}");
        }
    }

    #[test]
    fn keeps_context_and_ordinary_text() {
        assert_eq!(
            mask("Authorization: Bearer abcdefghijklmnopqrstuvwxyz"),
            format!("Authorization: Bearer {MASK}")
        );
        let pem = concat!(
            "a\n-----BEGIN PRIVATE",
            " KEY-----\nMIIE\n-----END PRIVATE",
            " KEY-----\nb"
        );
        assert_eq!(mask(pem), format!("a\n{MASK}\nb"));
        for plain in [
            "sk-short",
            "the task-runner ran",
            "git commit -m 'Add ghp support'",
            "-----BEGIN PUBLIC KEY-----",
            "bearer of bad news",
            "hippo: tokens model=claude-sonnet-5-5 input=2",
            "token: String,",
        ] {
            assert_eq!(mask(plain), plain);
        }
    }

    #[test]
    fn masks_prefixed_keys_of_any_issuer() {
        // Made up, and assembled at runtime like the cases above.
        let random = "aB3dE5gH7jK9".repeat(4);
        let hex = "a1b2c3d4".repeat(8);
        assert_eq!(
            mask(&format!("sent xai-{random} back")),
            format!("sent {MASK} back")
        );
        assert_eq!(mask(&format!("KEY=boat_{hex}")), format!("KEY={MASK}"));
        assert_eq!(mask(&format!("wxyz_ci_{random}")), MASK);
        assert_eq!(
            mask(&format!(r#""value":"xai-realtime-secret-12-{random}""#)),
            format!(r#""value":"{MASK}""#)
        );
    }

    #[test]
    fn masks_values_named_key() {
        let value = "0123abcdef".repeat(3);
        for (before, after) in [
            ("NZB_KEY=", ""),
            ("export AUTH_KEY=\"", "\""),
            ("api_key = ", ""),
            ("{\"api_key\": \"", "\"}"),
            ("{\"api_key\":\"", "\"}"),
            ("x-api-key: ", ""),
            ("https://host/api?mode=queue&apikey=", "&output=json"),
        ] {
            let text = format!("{before}{value}{after}");
            assert_eq!(mask(&text), format!("{before}{MASK}{after}"));
        }
        let mixed = "aB3dE5gH7jK9".repeat(2);
        assert_eq!(mask(&format!("?key={mixed}")), format!("?key={MASK}"));
    }

    #[test]
    fn keeps_near_misses() {
        let hex = "a1b2c3d4".repeat(8);
        let lower = "a1b2c3d4e5f6g7h8".repeat(2);
        let mixed = "aB3dE5gH7jK9".repeat(8);
        let ids = [
            // Call, tool, session and response ids.
            format!("search_{lower}"),
            format!("extract_{lower}"),
            format!("session_{}", &hex[..32]),
            format!("toolu_{}", &mixed[..24]),
            format!("rs_{}", &hex[..48]),
            format!("agent-{}-4e5f-6a7b-8c9d-{}", &hex[..8], &hex[..12]),
            // A Stripe invoice link.
            format!(
                "https://pay.stripe.com/invoice/acct_{}/live_{mixed}?s=ap",
                &mixed[..16]
            ),
            // Hashes.
            format!("sha256-{}=", &mixed[..43]),
            format!("/nix/store/{lower}-rust_hippo-0.1.0/bin/hippo"),
            format!("{lower}-procps-4.0.7"),
            format!("commit {}", &hex[..40]),
            format!("rev-{}", &hex[..40]),
            format!("docker-{hex}.scope"),
            format!("cache key: {hex}"),
            format!("{{\"key\":\"{hex}\"}}"),
            // Public ids and values given by reference.
            format!("nodekey:{hex}"),
            format!("public_key = {hex}"),
            "api_key: ${{ secrets.WXYZ_API_KEY }}".to_owned(),
            "key: process.env.WXYZ_API_KEY".to_owned(),
            "idempotency-key: 1a2b3c4d-1a2b-3c4d-5e6f-1a2b3c4d5e6f".to_owned(),
        ];
        for id in &ids {
            assert_eq!(&mask(id), id);
        }
        for plain in [
            "libri-review-web-desktop",
            "kit-2026-10-30-1432",
            "worker-sections-bottom-aligned-with-the-footer-on-1440-wide",
            "prompt_cache_key_retention_and_the_other_long_names_v2",
            "key: a_long_snake_case_name_for_version_2_of_it",
            "compact_summarized_and_also_written_as_one_lowercase_word",
        ] {
            assert_eq!(mask(plain), plain);
        }
    }
}
