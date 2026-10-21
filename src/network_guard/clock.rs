use super::model::Stamp;
use std::{fs::File, io::Read};

fn uptime_ms(value: &str) -> Result<u64, String> {
    let first = value.split_whitespace().next().ok_or("缺少单调启动时间")?;
    let (whole, fraction) = first.split_once('.').ok_or("启动时间格式无效")?;
    if whole.is_empty()
        || fraction.is_empty()
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|byte| byte.is_ascii_digit())
    {
        return Err("启动时间格式无效".into());
    }
    let whole = whole
        .parse::<u64>()
        .ok()
        .and_then(|value| value.checked_mul(1000))
        .ok_or("启动时间溢出")?;
    let fraction = format!("{fraction:0<3}")
        .chars()
        .take(3)
        .collect::<String>()
        .parse::<u64>()
        .map_err(|_| "启动时间小数无效")?;
    whole
        .checked_add(fraction)
        .ok_or_else(|| "启动时间溢出".into())
}

fn read(path: &str) -> Result<String, String> {
    let mut value = String::new();
    File::open(path)
        .and_then(|file| file.take(256).read_to_string(&mut value))
        .map_err(|_| "无法读取内核启动时钟")?;
    if value.len() >= 256 {
        return Err("内核时钟输出异常".into());
    }
    Ok(value)
}

pub fn current() -> Result<Stamp, String> {
    let boot = read("/proc/sys/kernel/random/boot_id")?;
    let boot = boot.trim();
    if !super::model::valid_boot(boot) {
        return Err("内核 boot ID 无效".into());
    }
    Ok(Stamp {
        boot_id: boot.into(),
        uptime_ms: uptime_ms(&read("/proc/uptime")?)?,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn uptime_parsing_is_integer_bounded_and_independent_of_wall_clock_time() {
        assert_eq!(super::uptime_ms("123.45 456.0\n").unwrap(), 123450);
        assert_eq!(super::uptime_ms("0.001 0").unwrap(), 1);
        assert!(super::uptime_ms("NaN 0").is_err());
        assert!(super::uptime_ms("-1.0 0").is_err());
        assert!(super::uptime_ms("18446744073709551615.99 0").is_err());
    }
}
