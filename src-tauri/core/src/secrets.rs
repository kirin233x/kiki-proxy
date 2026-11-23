//! 密钥存储（App 里是 macOS 钥匙串，测试里是内存）。

use anyhow::Result;

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, value: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}

#[derive(Default)]
pub struct MemorySecrets(parking_lot::Mutex<std::collections::HashMap<String, String>>);

impl SecretStore for MemorySecrets {
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.0.lock().get(key).cloned())
    }
    fn set(&self, key: &str, value: &str) -> Result<()> {
        self.0.lock().insert(key.into(), value.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().remove(key);
        Ok(())
    }
}

/// 钥匙串里各类密钥的命名
pub mod keys {
    pub fn endpoint_api_key(endpoint: &str) -> String {
        format!("endpoint/{endpoint}/api_key")
    }
    pub fn proxy_password(proxy: &str) -> String {
        format!("proxy/{proxy}/password")
    }
}
