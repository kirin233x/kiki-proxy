//! 插件凭证的缓存、刷新与重新登录。
//!
//! - 同一个端点同时只会有一个登录流程（多个请求同时发现过期，只弹一次登录）
//! - 上游返回鉴权失败时，只作废「导致失败的那个 token」，避免并发请求重复登录

use crate::plugin::{keys, Credential, Plugin, PluginCtx, SecretStore};
use anyhow::Result;
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;

pub struct Credentials {
    secrets: Arc<dyn SecretStore>,
    cache: Mutex<HashMap<String, Credential>>,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialStatus {
    pub logged_in: bool,
    pub expires_at: Option<i64>,
    pub expired: bool,
}

impl Credentials {
    pub fn new(secrets: Arc<dyn SecretStore>) -> Self {
        Credentials { secrets, cache: Mutex::new(HashMap::new()), locks: Mutex::new(HashMap::new()) }
    }

    fn lock_for(&self, endpoint: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks.lock().entry(endpoint.to_string()).or_default().clone()
    }

    fn cached(&self, endpoint: &str) -> Option<Credential> {
        if let Some(c) = self.cache.lock().get(endpoint) {
            return Some(c.clone());
        }
        let stored = self.secrets.get(&keys::endpoint_credential(endpoint)).ok().flatten()?;
        let c: Credential = serde_json::from_str(&stored).ok()?;
        self.cache.lock().insert(endpoint.to_string(), c.clone());
        Some(c)
    }

    fn store(&self, endpoint: &str, c: &Credential) {
        self.cache.lock().insert(endpoint.to_string(), c.clone());
        if let Ok(s) = serde_json::to_string(c) {
            if let Err(e) = self.secrets.set(&keys::endpoint_credential(endpoint), &s) {
                tracing::warn!("保存凭证到钥匙串失败：{e}");
            }
        }
    }

    /// 取一个可用的凭证：缓存 → 刷新 → 登录
    pub async fn get(&self, plugin: &dyn Plugin, ctx: &PluginCtx) -> Result<Credential> {
        let ep = &ctx.endpoint_id;
        if let Some(c) = self.cached(ep).filter(|c| !c.is_expired()) {
            return Ok(c);
        }
        let lock = self.lock_for(ep);
        let _g = lock.lock().await;
        // 拿到锁之后再看一次，可能别的请求已经登录好了
        let current = self.cached(ep);
        if let Some(c) = current.clone().filter(|c| !c.is_expired()) {
            return Ok(c);
        }
        if let Some(old) = current.filter(|c| c.refresh_token.is_some()) {
            match plugin.refresh(ctx, &old).await {
                Ok(c) => {
                    self.store(ep, &c);
                    return Ok(c);
                }
                Err(e) => tracing::info!("刷新凭证失败，重新登录：{e}"),
            }
        }
        let c = plugin.login(ctx).await?;
        self.store(ep, &c);
        Ok(c)
    }

    /// 上游说这个 token 不行了：如果缓存里还是它，就作废
    pub fn invalidate(&self, endpoint: &str, token: &str) {
        let mut cache = self.cache.lock();
        let matches = cache.get(endpoint).map(|c| c.token == token).unwrap_or(true);
        if matches {
            cache.remove(endpoint);
            let _ = self.secrets.delete(&keys::endpoint_credential(endpoint));
        }
    }

    /// 用户在界面上点「登录」：强制走一次完整登录
    pub async fn login(&self, plugin: &dyn Plugin, ctx: &PluginCtx) -> Result<Credential> {
        let lock = self.lock_for(&ctx.endpoint_id);
        let _g = lock.lock().await;
        let c = plugin.login(ctx).await?;
        self.store(&ctx.endpoint_id, &c);
        Ok(c)
    }

    pub fn logout(&self, endpoint: &str) {
        self.cache.lock().remove(endpoint);
        let _ = self.secrets.delete(&keys::endpoint_credential(endpoint));
    }

    pub fn status(&self, endpoint: &str) -> CredentialStatus {
        match self.cached(endpoint) {
            Some(c) => CredentialStatus { logged_in: true, expires_at: c.expires_at, expired: c.is_expired() },
            None => CredentialStatus { logged_in: false, expires_at: None, expired: false },
        }
    }
}
