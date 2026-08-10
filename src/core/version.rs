use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CoreVersion {
    major: u32,
    minor: u32,
    patch: u32,
}

impl CoreVersion {
    pub fn parse(value: &str) -> Result<Self, String> {
        let numbers = value
            .strip_prefix('v')
            .ok_or_else(|| format!("Mihomo version must start with v: {value}"))?;
        let parts = numbers.split('.').collect::<Vec<_>>();
        if parts.len() != 3
            || parts
                .iter()
                .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return Err(format!(
                "Mihomo version must use vMAJOR.MINOR.PATCH: {value}"
            ));
        }
        let version = Self {
            major: parts[0]
                .parse()
                .map_err(|_| format!("invalid Mihomo major version: {value}"))?,
            minor: parts[1]
                .parse()
                .map_err(|_| format!("invalid Mihomo minor version: {value}"))?,
            patch: parts[2]
                .parse()
                .map_err(|_| format!("invalid Mihomo patch version: {value}"))?,
        };
        if version.to_string() != value {
            return Err(format!("Mihomo version is not canonical: {value}"));
        }
        Ok(version)
    }

    pub fn from_mihomo_output(output: &str) -> Result<Self, String> {
        let candidates = output
            .split_whitespace()
            .filter(|word| {
                word.as_bytes().first() == Some(&b'v')
                    && word.as_bytes().get(1).is_some_and(u8::is_ascii_digit)
            })
            .map(Self::parse)
            .collect::<Result<Vec<_>, _>>()?;
        match candidates.as_slice() {
            [version] => Ok(*version),
            [] => Err(format!(
                "Mihomo version output does not contain vMAJOR.MINOR.PATCH: {}",
                output.trim()
            )),
            _ => Err("Mihomo version output contains multiple version tags".into()),
        }
    }

    pub(super) fn deb_version(self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl fmt::Display for CoreVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "v{}.{}.{}", self.major, self.minor, self.patch)
    }
}
