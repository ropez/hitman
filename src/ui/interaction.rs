use anyhow::Result;
use hitman::substitute::UserInteraction;

pub struct UiUserInteraction;

impl UserInteraction for UiUserInteraction {
    fn prompt(&self, key: &str, fallback: Option<&str>) -> Result<String> {
        todo!()
    }

    fn select(&self, key: &str, values: &[toml::Value]) -> Result<String> {
        todo!()
    }
}
