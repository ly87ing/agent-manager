//! 构建身份元数据透传：发布构建入口（scripts/build-macos.sh）在环境中给出
//! AGENT_MANAGER_BUILD_COMMIT（完整 HEAD SHA，dirty 构建带 -dirty 后缀）与
//! AGENT_MANAGER_BUILD_DATE（commit ISO 日期/RFC3339）。
//! 严格白名单：两值必须通过固定形状与字段范围校验才允许写入
//! cargo:rustc-env；缺失或不合法统一注入空串（消费端 fail-closed 为 null），
//! 告警只含变量名、不回显原值。白名单字符集天然阻断换行与 cargo 指令注入，
//! 也拒绝任何非身份形态的秘密内容。本脚本不执行 git、不读源码树、不引入
//! 任何额外环境内容。
//! 单元测试：cargo 不测试 build script，测试模块经
//! `rustc --edition 2021 --test build.rs` 编译运行；正常构建中被 cfg(test) 裁剪。

fn main() {
    emit_validated("AGENT_MANAGER_BUILD_COMMIT", is_valid_commit);
    emit_validated("AGENT_MANAGER_BUILD_DATE", is_valid_date);
}

/// 校验通过原样透传；缺失或不合法一律注入空串，不合法时告警且不回显原值。
fn emit_validated(var: &str, validate: fn(&str) -> bool) {
    println!("cargo:rerun-if-env-changed={var}");
    let value = std::env::var(var).unwrap_or_default();
    if value.is_empty() {
        println!("cargo:rustc-env={var}=");
    } else if validate(&value) {
        println!("cargo:rustc-env={var}={value}");
    } else {
        println!("cargo:rustc-env={var}=");
        println!("cargo:warning={var} 环境值形状不合法，已按缺失处理");
    }
}

/// 合法 commit：40 位小写 hex（git 输出恒为小写），可选 `-dirty` 后缀；
/// 其余字符（含换行/空白/大写）一律非法。
fn is_valid_commit(commit: &str) -> bool {
    let sha = commit.strip_suffix("-dirty").unwrap_or(commit);
    sha.len() == 40
        && sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// 合法日期：`YYYY-MM-DDTHH:MM:SS±HH:MM` 或 `Z` 结尾的固定形状与数字位，加
/// 字段范围校验（月份 01-12、日 01-31、时 00-23、分/秒 00-59、时区偏移同
/// 范围）。日历合法性（如 2 月 30）由消费端 RFC3339 语义校验（chrono）兜底。
fn is_valid_date(date: &str) -> bool {
    let b = date.as_bytes();
    let (body, offset) = match b.len() {
        25 if matches!(b[19], b'+' | b'-') && b[22] == b':' => (&b[..19], Some(&b[20..25])),
        20 if b[19] == b'Z' => (&b[..19], None),
        _ => return false,
    };
    if !is_datetime_body(body) {
        return false;
    }
    let field = |s: &[u8]| -> u32 { s.iter().fold(0u32, |acc, d| acc * 10 + (d - b'0') as u32) };
    let month = field(&body[5..7]);
    let day = field(&body[8..10]);
    let hour = field(&body[11..13]);
    let minute = field(&body[14..16]);
    let second = field(&body[17..19]);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return false;
    }
    if hour > 23 || minute > 59 || second > 59 {
        return false;
    }
    if let Some(off) = offset {
        // off 形如 `HH:MM`：冒号位置已由外层形状校验保证，这里只核数字位。
        if !off[0].is_ascii_digit()
            || !off[1].is_ascii_digit()
            || !off[3].is_ascii_digit()
            || !off[4].is_ascii_digit()
        {
            return false;
        }
        let off_hour = field(&off[0..2]);
        let off_minute = field(&off[3..5]);
        if off_hour > 23 || off_minute > 59 {
            return false;
        }
    }
    true
}

/// `YYYY-MM-DDTHH:MM:SS` 固定位置与数字位（调用方保证 b.len()==19）。
fn is_datetime_body(b: &[u8]) -> bool {
    debug_assert_eq!(b.len(), 19);
    let digit_at = |i: usize| b[i].is_ascii_digit();
    let digits = [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18];
    digits.into_iter().all(digit_at)
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && b[16] == b':'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_whitelist_accepts_clean_and_dirty() {
        let sha = "a".repeat(40);
        assert!(is_valid_commit(&sha));
        assert!(is_valid_commit(&format!("{sha}-dirty")));
    }

    #[test]
    fn commit_whitelist_rejects_injection_secret_and_malformed() {
        let sha = "a".repeat(40);
        for bad in [
            "8fa4274".to_string(),                               // 短 sha
            "A".repeat(40),                                      // 大写
            "g".repeat(40),                                      // 非 hex
            format!("{sha}\n"),                                  // 换行注入
            format!("{sha}\r\nX"),                               // CRLF 注入
            format!("{sha}\n cargo:rustc-env=X=secret"),         // cargo 指令注入
            format!("{sha} "),                                   // 空白
            format!("{sha}-DIRTY"),                              // 后缀形状不符
            format!("{sha}-dirtyx"),                             // 后缀冗余
            "sk-live-secret-token-0123456789abcdef".to_string(), // 非身份形态秘密
            String::new(),
        ] {
            assert!(!is_valid_commit(&bad), "commit={bad:?}");
        }
    }

    #[test]
    fn date_whitelist_accepts_valid_git_ci() {
        assert!(is_valid_date("2026-09-14T08:28:54+08:00"));
        assert!(is_valid_date("2026-09-14T00:28:54Z"));
        assert!(is_valid_date("2026-12-31T23:59:59-23:59"));
    }

    #[test]
    fn date_whitelist_rejects_bad_shape_injection_and_ranges() {
        for bad in [
            "2026-99-14T08:46:11+08:00",   // 月份 99
            "2026-00-14T08:46:11+08:00",   // 月份 00
            "2026-09-99T08:46:11+08:00",   // 日 99
            "2026-09-00T08:46:11+08:00",   // 日 00
            "2026-09-14T99:46:11+08:00",   // 时 99
            "2026-09-14T08:99:11+08:00",   // 分 99
            "2026-09-14T08:46:99+08:00",   // 秒 99
            "2026-09-14T08:46:11+ab:cd",   // 偏移非数字
            "2026-09-14T08:46:11+99:00",   // 偏移时 99
            "2026-09-14T08:46:11+08:99",   // 偏移分 99
            "2026-09-14T08:46:11",         // 缺时区
            "2026-09-14 08:46:11+08:00",   // 空格分隔
            "2026-09-14T08:46:11+0800",    // 偏移缺冒号
            "2026-09-14T08:46:11.5+08:00", // 小数秒
            "2026-09-14T08:46:11+08:00\n", // 换行注入
            "",
        ] {
            assert!(!is_valid_date(bad), "date={bad:?}");
        }
    }
}
