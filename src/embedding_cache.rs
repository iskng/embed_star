use parking_lot::RwLock;
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
use tracing::{debug, info};

/// Cache entry containing embedding data and metadata
#[derive(Debug, Clone)]
struct CacheEntry {
    embedding: Vec<f32>,
    model: String,
    created_at: Instant,
    last_accessed: Instant,
    access_count: u64,
}

/// LRU cache for embeddings with TTL support
pub struct EmbeddingCache {
    entries: Arc<RwLock<HashMap<String, CacheEntry>>>,
    access_order: Arc<RwLock<VecDeque<String>>>,
    max_size: usize,
    ttl: Duration,
}

impl EmbeddingCache {
    pub fn new(max_size: usize, ttl_seconds: u64) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::with_capacity(max_size))),
            access_order: Arc::new(RwLock::new(VecDeque::with_capacity(max_size))),
            max_size,
            ttl: Duration::from_secs(ttl_seconds),
        }
    }

    /// Generate a cache key from repo information
    pub fn cache_key(repo_full_name: &str, model: &str) -> String {
        format!("{}:{}", repo_full_name, model)
    }

    /// Get an embedding from cache if it exists and is not expired
    pub fn get(&self, key: &str) -> Option<(Vec<f32>, String)> {
        let mut entries = self.entries.write();
        let mut access_order = self.access_order.write();

        if let Some(entry) = entries.get_mut(key) {
            // Check if entry has expired
            if entry.created_at.elapsed() > self.ttl {
                debug!("Cache entry expired for key: {}", key);
                entries.remove(key);
                access_order.retain(|k| k != key);
                return None;
            }

            // Update access metadata
            entry.last_accessed = Instant::now();
            entry.access_count += 1;

            // Move to end of access order (most recently used)
            access_order.retain(|k| k != key);
            access_order.push_back(key.to_string());

            debug!(
                "Cache hit for key: {} (access count: {})",
                key, entry.access_count
            );

            Some((entry.embedding.clone(), entry.model.clone()))
        } else {
            None
        }
    }

    /// Put an embedding into the cache
    pub fn put(&self, key: String, embedding: Vec<f32>, model: String) {
        let mut entries = self.entries.write();
        let mut access_order = self.access_order.write();

        // Check if we need to evict old entries
        while entries.len() >= self.max_size {
            if let Some(oldest_key) = access_order.pop_front() {
                entries.remove(&oldest_key);
                debug!("Evicted cache entry: {}", oldest_key);
            }
        }

        // Insert new entry
        let entry = CacheEntry {
            embedding,
            model,
            created_at: Instant::now(),
            last_accessed: Instant::now(),
            access_count: 0,
        };

        entries.insert(key.clone(), entry);
        access_order.push_back(key.clone());

        debug!("Added cache entry: {} (cache size: {})", key, entries.len());
    }

    /// Remove expired entries from the cache
    pub fn evict_expired(&self) {
        let mut entries = self.entries.write();
        let mut access_order = self.access_order.write();
        let now = Instant::now();
        let mut expired_keys = Vec::new();

        for (key, entry) in entries.iter() {
            if now.duration_since(entry.created_at) > self.ttl {
                expired_keys.push(key.clone());
            }
        }

        let expired_count = expired_keys.len();
        
        for key in expired_keys {
            entries.remove(&key);
            access_order.retain(|k| k != &key);
        }

        if expired_count > 0 {
            info!("Evicted {} expired cache entries", expired_count);
        }
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        let entries = self.entries.read();
        let total_entries = entries.len();
        let total_memory = entries
            .values()
            .map(|e| e.embedding.len() * std::mem::size_of::<f32>())
            .sum::<usize>();

        let hit_count = entries.values().map(|e| e.access_count).sum::<u64>();

        CacheStats {
            total_entries,
            total_memory_bytes: total_memory,
            hit_count,
            max_size: self.max_size,
            ttl_seconds: self.ttl.as_secs(),
        }
    }

    /// Clear all entries from the cache
    pub fn clear(&self) {
        let mut entries = self.entries.write();
        let mut access_order = self.access_order.write();
        
        entries.clear();
        access_order.clear();
        
        info!("Cache cleared");
    }
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub total_entries: usize,
    pub total_memory_bytes: usize,
    pub hit_count: u64,
    pub max_size: usize,
    pub ttl_seconds: u64,
}

impl Default for EmbeddingCache {
    fn default() -> Self {
        // Default: 10k entries, 1 hour TTL
        Self::new(10_000, 3600)
    }
}

/// Background task to periodically clean up expired entries
pub async fn cache_cleanup_task(
    cache: Arc<EmbeddingCache>,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(300)); // Every 5 minutes

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                info!("Cache cleanup task shutting down");
                break;
            }
            _ = interval.tick() => {
                cache.evict_expired();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_basic_operations() {
        let cache = EmbeddingCache::new(2, 60);
        let embedding1 = vec![0.1, 0.2, 0.3];
        let embedding2 = vec![0.4, 0.5, 0.6];

        // Test put and get
        cache.put("key1".to_string(), embedding1.clone(), "model1".to_string());
        let result = cache.get("key1");
        assert!(result.is_some());
        let (retrieved, model) = result.unwrap();
        assert_eq!(retrieved, embedding1);
        assert_eq!(model, "model1");

        // Test cache miss
        assert!(cache.get("nonexistent").is_none());

        // Test LRU eviction
        cache.put("key2".to_string(), embedding2.clone(), "model2".to_string());
        cache.put("key3".to_string(), vec![0.7, 0.8, 0.9], "model3".to_string());
        
        // key1 should be evicted
        assert!(cache.get("key1").is_none());
        assert!(cache.get("key2").is_some());
        assert!(cache.get("key3").is_some());
    }

    #[test]
    fn test_cache_stats() {
        let cache = EmbeddingCache::new(100, 3600);
        
        // Add some entries
        for i in 0..5 {
            cache.put(
                format!("key{}", i),
                vec![0.1; 100],
                "model".to_string(),
            );
        }

        // Access some entries
        cache.get("key0");
        cache.get("key0");
        cache.get("key1");

        let stats = cache.stats();
        assert_eq!(stats.total_entries, 5);
        assert_eq!(stats.hit_count, 3);
        assert_eq!(stats.max_size, 100);
    }
}