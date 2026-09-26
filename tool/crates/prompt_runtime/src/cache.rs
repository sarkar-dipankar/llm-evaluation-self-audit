//! Caching layer for IR generation and provider responses.
//!
//! This module provides:
//! - Content-addressed IR caching (hash-based)
//! - Provider response caching for replay
//! - Disk persistence for cross-session caching
//! - TTL-based expiration

use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::SystemTime;

use prompt_ir::PromptIR;
use serde::{Deserialize, Serialize};

/// Content hash for cache keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentHash(u64);

impl ContentHash {
    /// Compute hash from content string.
    pub fn from_content(content: &str) -> Self {
        use std::collections::hash_map::DefaultHasher;
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        Self(hasher.finish())
    }

    /// Get the hash value.
    pub fn value(&self) -> u64 {
        self.0
    }
}

/// Cached IR entry with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedIR {
    /// The cached IR
    pub ir: PromptIR,
    /// Content hash that produced this IR
    pub content_hash: u64,
    /// Creation timestamp (Unix seconds)
    pub created_at: u64,
    /// Last access timestamp (Unix seconds)
    pub last_accessed: u64,
    /// Access count
    pub access_count: u64,
    /// Access sequence for LRU ordering (higher = more recent)
    #[serde(default)]
    pub access_sequence: u64,
}

/// Cached provider response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedResponse {
    /// The prompt that generated this response
    pub prompt_hash: u64,
    /// Provider name
    pub provider: String,
    /// Model used
    pub model: String,
    /// Response content
    pub response: String,
    /// Token count (if available)
    pub tokens_used: Option<u64>,
    /// Creation timestamp (Unix seconds)
    pub created_at: u64,
}

/// Configuration for the cache.
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Maximum number of IR entries to cache in memory
    pub max_ir_entries: usize,
    /// Maximum number of response entries to cache
    pub max_response_entries: usize,
    /// TTL for cached entries (in seconds)
    pub ttl_seconds: u64,
    /// Directory for disk persistence (None = memory only)
    pub cache_dir: Option<PathBuf>,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_ir_entries: 100,
            max_response_entries: 500,
            ttl_seconds: 3600, // 1 hour
            cache_dir: None,
        }
    }
}

/// In-memory cache with optional disk persistence.
pub struct Cache {
    config: CacheConfig,
    ir_cache: RwLock<HashMap<u64, CachedIR>>,
    response_cache: RwLock<HashMap<u64, CachedResponse>>,
    /// Monotonic sequence counter for LRU ordering
    sequence: std::sync::atomic::AtomicU64,
}

impl Cache {
    /// Create a new cache with the given configuration.
    pub fn new(config: CacheConfig) -> Self {
        let cache = Self {
            config,
            ir_cache: RwLock::new(HashMap::new()),
            response_cache: RwLock::new(HashMap::new()),
            sequence: std::sync::atomic::AtomicU64::new(0),
        };

        // Load from disk if configured
        if let Some(dir) = &cache.config.cache_dir {
            let _ = cache.load_from_disk(dir);
        }

        cache
    }

    /// Get next sequence number for LRU ordering.
    fn next_sequence(&self) -> u64 {
        self.sequence
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }

    /// Create a memory-only cache with default config.
    pub fn memory_only() -> Self {
        Self::new(CacheConfig::default())
    }

    /// Create a persistent cache in the given directory.
    pub fn persistent(cache_dir: PathBuf) -> Self {
        Self::new(CacheConfig {
            cache_dir: Some(cache_dir),
            ..Default::default()
        })
    }

    /// Get cached IR for the given content.
    pub fn get_ir(&self, content: &str) -> Option<PromptIR> {
        let hash = ContentHash::from_content(content);
        let now = current_unix_time();

        let mut cache = self.ir_cache.write().ok()?;
        if let Some(entry) = cache.get_mut(&hash.value()) {
            // Check TTL
            if now - entry.created_at > self.config.ttl_seconds {
                cache.remove(&hash.value());
                return None;
            }

            // Update access stats
            entry.last_accessed = now;
            entry.access_count += 1;
            entry.access_sequence = self.next_sequence();

            return Some(entry.ir.clone());
        }

        None
    }

    /// Cache IR for the given content.
    pub fn put_ir(&self, content: &str, ir: PromptIR) {
        let hash = ContentHash::from_content(content);
        let now = current_unix_time();

        let entry = CachedIR {
            ir,
            content_hash: hash.value(),
            created_at: now,
            last_accessed: now,
            access_count: 1,
            access_sequence: self.next_sequence(),
        };

        if let Ok(mut cache) = self.ir_cache.write() {
            // Evict if at capacity
            if cache.len() >= self.config.max_ir_entries {
                Self::evict_lru(&mut cache);
            }

            cache.insert(hash.value(), entry);
        }
    }

    /// Get cached response for the given prompt.
    pub fn get_response(&self, prompt: &str, provider: &str, model: &str) -> Option<String> {
        let hash = response_hash(prompt, provider, model);
        let now = current_unix_time();

        let cache = self.response_cache.read().ok()?;
        if let Some(entry) = cache.get(&hash) {
            // Check TTL
            if now - entry.created_at > self.config.ttl_seconds {
                return None;
            }

            return Some(entry.response.clone());
        }

        None
    }

    /// Cache a provider response.
    pub fn put_response(
        &self,
        prompt: &str,
        provider: &str,
        model: &str,
        response: String,
        tokens: Option<u64>,
    ) {
        let hash = response_hash(prompt, provider, model);
        let now = current_unix_time();

        let entry = CachedResponse {
            prompt_hash: ContentHash::from_content(prompt).value(),
            provider: provider.to_string(),
            model: model.to_string(),
            response,
            tokens_used: tokens,
            created_at: now,
        };

        if let Ok(mut cache) = self.response_cache.write() {
            // Evict if at capacity
            if cache.len() >= self.config.max_response_entries {
                self.evict_oldest_response(&mut cache);
            }

            cache.insert(hash, entry);
        }
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        let ir_count = self.ir_cache.read().map(|c| c.len()).unwrap_or(0);
        let response_count = self.response_cache.read().map(|c| c.len()).unwrap_or(0);

        CacheStats {
            ir_entries: ir_count,
            response_entries: response_count,
            max_ir_entries: self.config.max_ir_entries,
            max_response_entries: self.config.max_response_entries,
        }
    }

    /// Clear all cached entries.
    pub fn clear(&self) {
        if let Ok(mut cache) = self.ir_cache.write() {
            cache.clear();
        }
        if let Ok(mut cache) = self.response_cache.write() {
            cache.clear();
        }
    }

    /// Persist cache to disk.
    pub fn persist(&self) -> io::Result<()> {
        if let Some(dir) = &self.config.cache_dir {
            fs::create_dir_all(dir)?;

            // Save IR cache
            if let Ok(cache) = self.ir_cache.read() {
                let ir_path = dir.join("ir_cache.json");
                let json = serde_json::to_string(&*cache)?;
                fs::write(ir_path, json)?;
            }

            // Save response cache
            if let Ok(cache) = self.response_cache.read() {
                let response_path = dir.join("response_cache.json");
                let json = serde_json::to_string(&*cache)?;
                fs::write(response_path, json)?;
            }
        }

        Ok(())
    }

    /// Load cache from disk.
    fn load_from_disk(&self, dir: &Path) -> io::Result<()> {
        // Load IR cache
        let ir_path = dir.join("ir_cache.json");
        if ir_path.exists() {
            let json = fs::read_to_string(&ir_path)?;
            if let Ok(loaded) = serde_json::from_str::<HashMap<u64, CachedIR>>(&json) {
                if let Ok(mut cache) = self.ir_cache.write() {
                    *cache = loaded;
                }
            }
        }

        // Load response cache
        let response_path = dir.join("response_cache.json");
        if response_path.exists() {
            let json = fs::read_to_string(&response_path)?;
            if let Ok(loaded) = serde_json::from_str::<HashMap<u64, CachedResponse>>(&json) {
                if let Ok(mut cache) = self.response_cache.write() {
                    *cache = loaded;
                }
            }
        }

        Ok(())
    }

    /// Evict least recently used IR entry.
    fn evict_lru(cache: &mut HashMap<u64, CachedIR>) {
        if let Some((&oldest_key, _)) = cache.iter().min_by_key(|(_, v)| v.access_sequence) {
            cache.remove(&oldest_key);
        }
    }

    /// Evict oldest response entry.
    fn evict_oldest_response(&self, cache: &mut HashMap<u64, CachedResponse>) {
        if let Some((&oldest_key, _)) = cache.iter().min_by_key(|(_, v)| v.created_at) {
            cache.remove(&oldest_key);
        }
    }
}

/// Cache statistics.
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub ir_entries: usize,
    pub response_entries: usize,
    pub max_ir_entries: usize,
    pub max_response_entries: usize,
}

/// Compute hash for response lookup.
fn response_hash(prompt: &str, provider: &str, model: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    let mut hasher = DefaultHasher::new();
    prompt.hash(&mut hasher);
    provider.hash(&mut hasher);
    model.hash(&mut hasher);
    hasher.finish()
}

/// Get current Unix timestamp.
fn current_unix_time() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Global cache instance.
static GLOBAL_CACHE: std::sync::OnceLock<Cache> = std::sync::OnceLock::new();

/// Get or initialize the global cache.
pub fn global_cache() -> &'static Cache {
    GLOBAL_CACHE.get_or_init(Cache::memory_only)
}

/// Initialize global cache with custom config.
pub fn init_global_cache(config: CacheConfig) {
    let _ = GLOBAL_CACHE.set(Cache::new(config));
}

#[cfg(test)]
mod tests {
    use super::*;
    use prompt_ir::{IRKind, IRNode, IRNodeMeta, TextRange};

    fn sample_ir() -> PromptIR {
        PromptIR {
            uri: "test://example".to_string(),
            version: 1,
            nodes: vec![IRNode {
                id: "RULE:TEST".to_string(),
                kind: IRKind::Rule,
                label: Some("Test rule".to_string()),
                range: TextRange::default(),
                meta: IRNodeMeta::default(),
            }],
            meta: None,
        }
    }

    #[test]
    fn test_content_hash() {
        let hash1 = ContentHash::from_content("hello world");
        let hash2 = ContentHash::from_content("hello world");
        let hash3 = ContentHash::from_content("different content");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_ir_cache() {
        let cache = Cache::memory_only();

        let content = "test prompt content";
        let ir = sample_ir();

        // Cache miss
        assert!(cache.get_ir(content).is_none());

        // Cache put
        cache.put_ir(content, ir.clone());

        // Cache hit
        let cached = cache.get_ir(content);
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().uri, ir.uri);
    }

    #[test]
    fn test_response_cache() {
        let cache = Cache::memory_only();

        let prompt = "test prompt";
        let provider = "openai";
        let model = "gpt-4";
        let response = "test response".to_string();

        // Cache miss
        assert!(cache.get_response(prompt, provider, model).is_none());

        // Cache put
        cache.put_response(prompt, provider, model, response.clone(), Some(100));

        // Cache hit
        let cached = cache.get_response(prompt, provider, model);
        assert!(cached.is_some());
        assert_eq!(cached.unwrap(), response);
    }

    #[test]
    fn test_cache_stats() {
        let cache = Cache::memory_only();

        let stats = cache.stats();
        assert_eq!(stats.ir_entries, 0);
        assert_eq!(stats.response_entries, 0);

        cache.put_ir("content1", sample_ir());
        cache.put_ir("content2", sample_ir());
        cache.put_response("prompt", "provider", "model", "response".to_string(), None);

        let stats = cache.stats();
        assert_eq!(stats.ir_entries, 2);
        assert_eq!(stats.response_entries, 1);
    }

    #[test]
    fn test_cache_clear() {
        let cache = Cache::memory_only();

        cache.put_ir("content", sample_ir());
        cache.put_response("prompt", "provider", "model", "response".to_string(), None);

        cache.clear();

        let stats = cache.stats();
        assert_eq!(stats.ir_entries, 0);
        assert_eq!(stats.response_entries, 0);
    }

    #[test]
    fn test_lru_eviction() {
        let config = CacheConfig {
            max_ir_entries: 2,
            ..Default::default()
        };
        let cache = Cache::new(config);

        // Fill cache
        cache.put_ir("content1", sample_ir());
        cache.put_ir("content2", sample_ir());

        // Access content1 to make it more recent
        let _ = cache.get_ir("content1");

        // Add third entry, should evict content2 (least recently used)
        cache.put_ir("content3", sample_ir());

        assert!(cache.get_ir("content1").is_some());
        assert!(cache.get_ir("content2").is_none()); // Evicted
        assert!(cache.get_ir("content3").is_some());
    }
}
