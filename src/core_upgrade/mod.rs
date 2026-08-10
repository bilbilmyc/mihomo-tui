mod activation;
mod upgrade;

#[cfg(test)]
mod tests;

pub fn upgrade() -> Result<String, String> {
    upgrade::upgrade()
}
