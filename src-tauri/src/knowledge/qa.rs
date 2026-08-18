//! RAG question-answering layer (P22 + P23).
//!
//! `KnowledgeQAService` is the public face of the knowledge base: it delegates
//! storage/retrieval to a [`KnowledgeStore`] and adds LLM post-processing. The
//! boundary is sharp — the store never touches an LLM/embedder, the QA layer
//! never writes SQL (the "storage layer doesn't touch LLM, QA layer doesn't
//! touch SQL" split decided during design review; G1 in the P23 precheck).
//!
//! ## LLM dependency (R1)
//!
//! The design document originally referenced a future `P23 ModelAdapter`. That
//! module does not exist; this implementation depends on the **live** `LlmProvider`
//! trait (defined in P13, with 4 real callers). `ask` is `#[async_trait]` because
//! `LlmProvider::generate` is async.
//!
//! ## Embedding dependency (L5 / P23)
//!
//! Semantic retrieval reuses the **live** `EmbeddingService` (P11). It is held
//! here in the QA layer (never in the store). `embed`/`embed_batch` are async and
//! lazy-load the model, so `embed_document`/`embed_all` are async too (the design
//! doc's "sync embed_*" was based on a wrong assumption that they were sync — see
//! precheck finding A).

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;

use crate::knowledge::error::KnowledgeResult;
use crate::knowledge::store::{
    DocumentSearchResult, KnowledgeStore, SearchResult, SemanticSearchResult, SqliteKnowledgeStore,
};
use crate::knowledge::EmbeddingService;
use crate::router::model_router::ModelRouter;
use crate::router::LlmProvider;

/// Maximum length (Unicode chars) of a source `snippet` returned to the caller.
pub const MAX_SNIPPET_LEN: usize = 200;

/// A cited source attached to a generated answer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Source {
    pub document_id: String,
    pub document_name: String,
    /// 0-based index of the cited chunk within its source document.
    pub chunk_index: usize,
    /// Truncated chunk text (≤ `MAX_SNIPPET_LEN` chars) for display.
    pub snippet: String,
}

/// Retrieval strategy for [`KnowledgeQAService::ask`] (P23).
///
/// - `Keyword` — pure FTS5 + BM25 (P22 path).
/// - `Semantic` — pure vector retrieval (P23 path).
/// - `Auto` — simple default: if any chunk is embedded, use `Semantic`, else
///   fall back to `Keyword`. This is **not** TurboAuto's full L1 routing
///   (TurboAuto is 0-hit in the codebase; P23 does not assume it exists).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetrievalStrategy {
    Keyword,
    Semantic,
    Auto,
}

/// Transparency metadata attached to a QA response (L4 hook; reserved for
/// TurboAuto). `engine` is `"keyword"` or `"semantic"` (not `"turboauto"` — that
/// belongs to a future stage).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct QAMeta {
    pub engine: String,
    pub reason: String,
}

/// A full QA response: the LLM answer plus the sources it was grounded in, plus
/// transparency `meta` (P23).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct QAResponse {
    /// The LLM-generated answer (includes citation markers like `[1]`).
    pub answer: String,
    /// Sources the answer was grounded in (for front-end citation display).
    pub sources: Vec<Source>,
    /// Transparency metadata: which engine answered and why.
    pub meta: QAMeta,
}

/// A normalized retrieved chunk, independent of which retrieval path produced it.
/// Lets `ask` build sources/prompts from keyword and semantic results uniformly.
struct Retrieved {
    document_id: String,
    document_name: String,
    chunk_index: usize,
    content: String,
}

/// The knowledge QA contract.
#[async_trait]
pub trait KnowledgeQAService: Send + Sync {
    /// Import a document (delegated to the store).
    fn import_document(
        &self,
        path: &Path,
        name: &str,
        content: &str,
    ) -> KnowledgeResult<crate::knowledge::store::Document>;

    /// List all documents (delegated to the store).
    fn list_documents(&self) -> KnowledgeResult<Vec<crate::knowledge::store::Document>>;

    /// Delete a document (delegated to the store).
    fn delete_document(&self, id: &str) -> KnowledgeResult<()>;

    /// Pure keyword search, no LLM (delegated to the store).
    fn search_only(&self, query: &str, limit: usize) -> KnowledgeResult<Vec<SearchResult>>;

    /// Answer a question using the chosen retrieval strategy.
    ///
    /// - When retrieval finds nothing, returns an "未找到相关内容" answer **without
    ///   calling the LLM** (saves tokens; acceptance #7).
    /// - LLM timeout / failure surfaces as `KnowledgeError::Llm`.
    /// - An empty LLM answer surfaces as `KnowledgeError::EmptyAnswer`.
    /// - `meta` records which engine answered and why (acceptance #11).
    async fn ask(
        &self,
        query: &str,
        top_k: usize,
        strategy: RetrievalStrategy,
        model: Option<&str>,
        skill_preferred: Option<&str>,
        agent_default: Option<&str>,
    ) -> KnowledgeResult<QAResponse>;

    /// Embed every chunk of one document (async; lazy-loads the model). Returns
    /// the number of chunks embedded.
    async fn embed_document(&self, document_id: &str) -> KnowledgeResult<usize>;

    /// Embed every not-yet-embedded chunk (async; lazy-loads the model). Returns
    /// the number of chunks embedded.
    async fn embed_all(&self) -> KnowledgeResult<usize>;

    /// `true` if any chunk still lacks an embedding.
    fn has_unembedded_chunks(&self) -> KnowledgeResult<bool>;

    /// Semantic search by free-text query: embeds the query, then delegates to
    /// the store (which does the pure cosine scan). Requires a loaded model.
    async fn search_semantic(
        &self,
        query: &str,
        top_k: usize,
    ) -> KnowledgeResult<Vec<SemanticSearchResult>>;

    /// Filename discovery (exact / bigram match on `documents.name`), delegated
    /// to the store. No model needed.
    fn search_documents(
        &self,
        query: &str,
        limit: usize,
    ) -> KnowledgeResult<Vec<DocumentSearchResult>>;
}

/// Default number of chunks retrieved when `top_k` is 0.
const DEFAULT_TOP_K: usize = 5;

/// Concrete QA service backed by SQLite storage + an `LlmProvider` + an
/// `EmbeddingService` + a `ModelRouter` (P24 multi-model routing).
pub struct SqliteKnowledgeQA {
    store: Arc<SqliteKnowledgeStore>,
    provider: Arc<dyn LlmProvider>,
    embedder: Arc<EmbeddingService>,
    /// P24 model-routing layer. Resolves the `LlmProvider` used by `ask` when a
    /// model preference is supplied; falls back to `provider` otherwise.
    router: Arc<ModelRouter>,
}

impl SqliteKnowledgeQA {
    /// Build a QA service from a store, an LLM provider, an embedding service,
    /// and a model router.
    ///
    /// `provider` is the default LLM used when `ask` is called without a model
    /// preference; `router` drives the P24 priority + fallback selection.
    pub fn new(
        store: Arc<SqliteKnowledgeStore>,
        provider: Arc<dyn LlmProvider>,
        embedder: Arc<EmbeddingService>,
        router: Arc<ModelRouter>,
    ) -> Self {
        Self {
            store,
            provider,
            embedder,
            router,
        }
    }

    /// Access the underlying store (e.g. for direct inspection in tests).
    pub fn store(&self) -> &SqliteKnowledgeStore {
        &self.store
    }

    /// Access the embedding service (e.g. for direct inspection in tests).
    pub fn embedder(&self) -> &EmbeddingService {
        &self.embedder
    }
}

#[async_trait]
impl KnowledgeQAService for SqliteKnowledgeQA {
    fn import_document(
        &self,
        path: &Path,
        name: &str,
        content: &str,
    ) -> KnowledgeResult<crate::knowledge::store::Document> {
        self.store.import_document(path, name, content)
    }

    fn list_documents(&self) -> KnowledgeResult<Vec<crate::knowledge::store::Document>> {
        self.store.list_documents()
    }

    fn delete_document(&self, id: &str) -> KnowledgeResult<()> {
        self.store.delete_document(id)
    }

    fn search_only(&self, query: &str, limit: usize) -> KnowledgeResult<Vec<SearchResult>> {
        self.store.search_only(query, limit)
    }

    async fn ask(
        &self,
        query: &str,
        top_k: usize,
        strategy: RetrievalStrategy,
        model: Option<&str>,
        skill_preferred: Option<&str>,
        agent_default: Option<&str>,
    ) -> KnowledgeResult<QAResponse> {
        let top_k = if top_k == 0 { DEFAULT_TOP_K } else { top_k };

        // Resolve which engine to use. `Auto` picks semantic iff any chunk is
        // embedded (so a fresh, un-vectorized base correctly falls back to
        // keyword — acceptance #9).
        let engine = match strategy {
            RetrievalStrategy::Keyword => "keyword",
            RetrievalStrategy::Semantic => "semantic",
            RetrievalStrategy::Auto => {
                if self.store.embedded_chunk_count()? > 0 {
                    "semantic"
                } else {
                    "keyword"
                }
            }
        };

        // P24: resolve the LLM provider via the model router. When no model is
        // preferred, fall back to the legacy fixed provider. `model` (Task
        // preference) takes priority over skill/agent preferences inside the
        // router's chain.
        let (provider, model_id): (Arc<dyn LlmProvider>, String) =
            if model.is_some() || skill_preferred.is_some() || agent_default.is_some() {
                let resolved = self.router.resolve(
                    model,
                    skill_preferred,
                    agent_default,
                    crate::router::model_router::SelectionStrategy::Priority,
                )?;
                let mid = resolved.model_name().to_string();
                (resolved, mid)
            } else {
                (
                    self.provider.clone(),
                    self.provider.model_name().to_string(),
                )
            };

        let retrieved: Vec<Retrieved> = match engine {
            "keyword" => self
                .store
                .search_only(query, top_k)?
                .into_iter()
                .map(|r| Retrieved {
                    document_id: r.document_id,
                    document_name: r.document_name,
                    chunk_index: r.chunk_index,
                    content: r.content,
                })
                .collect(),
            "semantic" => {
                let qvec = self.embedder.embed(query).await.map_err(|e| {
                    crate::knowledge::error::KnowledgeError::Embedding(e.to_string())
                })?;
                self.store
                    .search_semantic(&qvec, top_k)?
                    .into_iter()
                    .map(|r| Retrieved {
                        document_id: r.document_id,
                        document_name: r.document_name,
                        chunk_index: r.chunk_index,
                        content: r.content,
                    })
                    .collect()
            }
            _ => unreachable!(),
        };

        let reason = match (strategy, engine) {
            (RetrievalStrategy::Keyword, _) => "user selected keyword".to_string(),
            (RetrievalStrategy::Semantic, _) => "user selected semantic".to_string(),
            (RetrievalStrategy::Auto, "semantic") => "auto: embeddings present".to_string(),
            (RetrievalStrategy::Auto, "keyword") => {
                "auto: no embeddings, fallback to keyword".to_string()
            }
            _ => unreachable!(),
        };

        // P24: surface the *actual* model used (router-resolved), not just the
        // retrieval engine, so callers can see which model answered.
        let model_reason = if model_id == self.provider.model_name() {
            "default provider".to_string()
        } else {
            format!("model: {model_id}")
        };
        let meta_reason = format!("{model_reason}; {reason}");

        if retrieved.is_empty() {
            return Ok(QAResponse {
                answer: "未找到相关内容。".to_string(),
                sources: Vec::new(),
                meta: QAMeta {
                    engine: model_id,
                    reason: meta_reason,
                },
            });
        }

        let sources: Vec<Source> = retrieved
            .iter()
            .map(|r| Source {
                document_id: r.document_id.clone(),
                document_name: r.document_name.clone(),
                chunk_index: r.chunk_index,
                snippet: snippet_of(&r.content, MAX_SNIPPET_LEN),
            })
            .collect();

        let prompt = build_prompt(query, &retrieved);
        let raw = provider.generate(&prompt).await?;
        let answer = raw.trim();
        if answer.is_empty() {
            return Err(crate::knowledge::error::KnowledgeError::EmptyAnswer);
        }
        Ok(QAResponse {
            answer: answer.to_string(),
            sources,
            meta: QAMeta {
                engine: model_id,
                reason: meta_reason,
            },
        })
    }

    async fn embed_document(&self, document_id: &str) -> KnowledgeResult<usize> {
        let chunks = self.store.chunks_of_document(document_id)?;
        if chunks.is_empty() {
            return Ok(0);
        }
        let texts: Vec<String> = chunks.iter().map(|(_, c)| c.clone()).collect();
        let vectors = self
            .embedder
            .embed_batch(&texts)
            .await
            .map_err(|e| crate::knowledge::error::KnowledgeError::Embedding(e.to_string()))?;
        for ((chunk_id, _), vec) in chunks.iter().zip(vectors.iter()) {
            self.store.write_embedding(*chunk_id, vec)?;
        }
        Ok(chunks.len())
    }

    async fn embed_all(&self) -> KnowledgeResult<usize> {
        let chunks = self.store.unembedded_chunks()?;
        if chunks.is_empty() {
            return Ok(0);
        }
        let texts: Vec<String> = chunks.iter().map(|(_, c)| c.clone()).collect();
        let vectors = self
            .embedder
            .embed_batch(&texts)
            .await
            .map_err(|e| crate::knowledge::error::KnowledgeError::Embedding(e.to_string()))?;
        for ((chunk_id, _), vec) in chunks.iter().zip(vectors.iter()) {
            self.store.write_embedding(*chunk_id, vec)?;
        }
        Ok(chunks.len())
    }

    fn has_unembedded_chunks(&self) -> KnowledgeResult<bool> {
        Ok(self.store.unembedded_chunk_count()? > 0)
    }

    async fn search_semantic(
        &self,
        query: &str,
        top_k: usize,
    ) -> KnowledgeResult<Vec<SemanticSearchResult>> {
        let qvec = self
            .embedder
            .embed(query)
            .await
            .map_err(|e| crate::knowledge::error::KnowledgeError::Embedding(e.to_string()))?;
        self.store.search_semantic(&qvec, top_k)
    }

    fn search_documents(
        &self,
        query: &str,
        limit: usize,
    ) -> KnowledgeResult<Vec<DocumentSearchResult>> {
        self.store.search_documents(query, limit)
    }
}

/// Truncate `content` to at most `max` chars, appending an ellipsis if cut.
fn snippet_of(content: &str, max: usize) -> String {
    let chars: Vec<char> = content.chars().collect();
    if chars.len() <= max {
        content.to_string()
    } else {
        let mut s: String = chars[..max].iter().collect();
        s.push('…');
        s
    }
}

/// Assemble the RAG prompt (design doc §3.3): retrieved fragments + the question,
/// with explicit instructions to cite sources and never fabricate.
fn build_prompt(query: &str, results: &[Retrieved]) -> String {
    let mut p = String::new();
    p.push_str("你是一个知识库助手。请基于以下检索到的文档片段回答用户的问题。\n\n");
    p.push_str("如果检索到的片段不足以回答问题，请明确告诉用户“检索到的内容不足以回答这个问题”，不要编造信息。\n\n");
    p.push_str("文档片段：\n");
    for (i, r) in results.iter().enumerate() {
        p.push_str(&format!("[{}] 文件名：{}\n", i + 1, r.document_name));
        p.push_str(&format!("    内容：{}\n", r.content));
    }
    p.push('\n');
    p.push_str(&format!("用户问题：{query}\n\n"));
    p.push_str("请基于以上片段回答，并在回答中标注引用来源（用 [1] 或 [2] 等编号）。");
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::settings::EmbeddingConfig;
    use crate::knowledge::store::Document;
    use crate::router::{MockLlmProvider, ModelSize};
    use crate::types::LlmError;
    use std::sync::Arc;

    fn make_embedding_config() -> EmbeddingConfig {
        EmbeddingConfig {
            model: "all-MiniLM-L6-v2".to_string(),
            max_batch_size: 32,
            idle_timeout_secs: 0, // never unload in tests
            cache_dir: String::new(),
            high_threshold: 0.82,
            low_threshold: 0.65,
        }
    }

    // P24: build a QA service wired to a real ModelRouter (over a ConfigManager
    // with the default sample models). The mock LLM provider uses the same name
    // as the config's default model ("deepseek-chat"), so `meta.engine` reflects
    // the router-resolved default when `ask` is called without a model pref.
    async fn temp_qa() -> (
        tempfile::TempDir,
        SqliteKnowledgeQA,
        Arc<MockLlmProvider>,
        Arc<ModelRouter>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("knowledge.db");
        let store = Arc::new(SqliteKnowledgeStore::open(&db).unwrap());
        // Name the mock after the config's default model ("deepseek-chat") so that
        // `meta.engine` reflects the default when `ask` is called without a
        // model preference (the no-preference path uses the held mock, not the
        // router — see `ask`).
        let provider = Arc::new(
            MockLlmProvider::single("工作流引擎支持 DAG 拓扑排序 [1]。".to_string())
                .with_name("deepseek-chat"),
        );
        let embedder = Arc::new(EmbeddingService::new(make_embedding_config()));
        let cfg = Arc::new(
            crate::config::ConfigManager::init_with_paths(Some(dir.path()))
                .await
                .unwrap(),
        );
        let router = Arc::new(ModelRouter::new(
            cfg,
            Arc::new(crate::router::health::InMemoryHealthChecker::new()),
        ));
        let qa = SqliteKnowledgeQA::new(store, provider.clone(), embedder, router.clone());
        (dir, qa, provider, router)
    }

    const ZH_DOC: &str = "工作流引擎支持 DAG 拓扑排序，采用 petgraph 实现。\n\n缓存策略采用哈希键精确匹配，中间结果落盘复用。";

    // Acceptance #6 / #7 / #11 migrated: ask(Keyword) returns an answer with
    // citations + sources + meta.engine = the router-resolved default model.
    #[tokio::test]
    async fn test_qa_with_sources() {
        let (_dir, qa, _mock, _router) = temp_qa().await;
        qa.import_document(Path::new("/docs/wf.md"), "wf.md", ZH_DOC)
            .unwrap();
        let resp = qa
            .ask(
                "工作流引擎是怎么实现的？",
                5,
                RetrievalStrategy::Keyword,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        assert!(!resp.answer.is_empty());
        assert!(
            resp.answer.contains('['),
            "answer should carry citation markers"
        );
        assert!(!resp.sources.is_empty());
        assert!(resp.sources[0].snippet.len() <= MAX_SNIPPET_LEN);
        assert!(resp.sources[0].document_name == "wf.md");
        assert_eq!(resp.meta.engine, "deepseek-chat");
        assert_eq!(resp.meta.reason, "default provider; user selected keyword");
    }

    // Acceptance #7: retrieval finds nothing -> no LLM call, "未找到" returned.
    #[tokio::test]
    async fn test_qa_no_results_no_llm_call() {
        let (_dir, qa, mock, _router) = temp_qa().await;
        qa.import_document(Path::new("/docs/wf.md"), "wf.md", ZH_DOC)
            .unwrap();
        // "量子" is not present in the doc -> 0 hits.
        let resp = qa
            .ask(
                "量子纠缠如何计算",
                5,
                RetrievalStrategy::Keyword,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(resp.answer, "未找到相关内容。");
        assert!(resp.sources.is_empty());
        assert_eq!(resp.meta.engine, "deepseek-chat");
        // The mock provider must NOT have been called.
        assert_eq!(
            mock.call_count(),
            0,
            "LLM must not be called when retrieval is empty"
        );
    }

    // Acceptance #8: LLM timeout -> friendly error (assertable variant).
    struct TimeoutProvider;
    #[async_trait]
    impl LlmProvider for TimeoutProvider {
        async fn generate(&self, _prompt: &str) -> crate::types::LlmResult<String> {
            Err(LlmError::Timeout)
        }
        fn model_name(&self) -> &str {
            "timeout-provider"
        }
        fn model_size(&self) -> ModelSize {
            ModelSize::Small
        }
    }

    async fn qa_with(provider: Arc<dyn LlmProvider>) -> SqliteKnowledgeQA {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("knowledge.db");
        let store = Arc::new(SqliteKnowledgeStore::open(&db).unwrap());
        let embedder = Arc::new(EmbeddingService::new(make_embedding_config()));
        let cfg = Arc::new(
            crate::config::ConfigManager::init_with_paths(Some(dir.path()))
                .await
                .unwrap(),
        );
        let router = Arc::new(ModelRouter::new(
            cfg,
            Arc::new(crate::router::health::InMemoryHealthChecker::new()),
        ));
        SqliteKnowledgeQA::new(store, provider, embedder, router)
    }

    #[tokio::test]
    async fn test_qa_timeout() {
        let qa = qa_with(Arc::new(TimeoutProvider)).await;
        qa.import_document(Path::new("/docs/wf.md"), "wf.md", ZH_DOC)
            .unwrap();
        let result = qa
            .ask(
                "工作流引擎是怎么实现的？",
                5,
                RetrievalStrategy::Keyword,
                None,
                None,
                None,
            )
            .await;
        assert!(matches!(
            result,
            Err(crate::knowledge::error::KnowledgeError::Llm(
                LlmError::Timeout
            ))
        ));
    }

    // LLM returns empty -> EmptyAnswer.
    #[tokio::test]
    async fn test_qa_empty_answer() {
        let qa = qa_with(Arc::new(MockLlmProvider::single(String::new()))).await;
        qa.import_document(Path::new("/docs/wf.md"), "wf.md", ZH_DOC)
            .unwrap();
        let result = qa
            .ask(
                "工作流引擎是怎么实现的？",
                5,
                RetrievalStrategy::Keyword,
                None,
                None,
                None,
            )
            .await;
        assert!(matches!(
            result,
            Err(crate::knowledge::error::KnowledgeError::EmptyAnswer)
        ));
    }

    // Delegation: list_documents passes through to the store.
    #[tokio::test]
    async fn test_qa_delegates_storage() {
        let (_dir, qa, _mock, _router) = temp_qa().await;
        qa.import_document(Path::new("/docs/wf.md"), "wf.md", ZH_DOC)
            .unwrap();
        let docs: Vec<Document> = qa.list_documents().unwrap();
        assert_eq!(docs.len(), 1);
        qa.delete_document(&docs[0].id).unwrap();
        assert!(qa.list_documents().unwrap().is_empty());
    }

    // Acceptance #7 (explicit): ask with RetrievalStrategy::Keyword returns the
    // keyword path; meta.engine is the router-resolved default model.
    #[tokio::test]
    async fn test_ask_keyword_strategy() {
        let (_dir, qa, _mock, _router) = temp_qa().await;
        qa.import_document(Path::new("/docs/wf.md"), "wf.md", ZH_DOC)
            .unwrap();
        let resp = qa
            .ask(
                "工作流引擎是怎么实现的？",
                5,
                RetrievalStrategy::Keyword,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(resp.meta.engine, "deepseek-chat");
        assert!(!resp.answer.is_empty());
        assert!(!resp.sources.is_empty());
    }

    // Acceptance #9 (keyword branch): ask(Auto) with no embeddings falls back to
    // keyword. No model load required, so this runs in CI.
    #[tokio::test]
    async fn test_ask_auto_fallback_to_keyword() {
        let (_dir, qa, _mock, _router) = temp_qa().await;
        qa.import_document(Path::new("/docs/wf.md"), "wf.md", ZH_DOC)
            .unwrap();
        // Fresh base: no chunk has an embedding yet.
        assert!(qa.has_unembedded_chunks().unwrap());
        let resp = qa
            .ask(
                "工作流引擎是怎么实现的？",
                5,
                RetrievalStrategy::Auto,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(resp.meta.engine, "deepseek-chat");
        assert_eq!(
            resp.meta.reason,
            "default provider; auto: no embeddings, fallback to keyword"
        );
        assert!(!resp.answer.is_empty());
    }

    // Filename discovery via the QA layer (delegates to the store; no model).
    #[tokio::test]
    async fn test_qa_search_documents() {
        let (_dir, qa, _mock, _router) = temp_qa().await;
        qa.import_document(
            Path::new("/docs/工作流引擎设计.md"),
            "工作流引擎设计.md",
            ZH_DOC,
        )
        .unwrap();
        let hits = qa.search_documents("引擎", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].document_name, "工作流引擎设计.md");
    }

    // `embed_all` without a loadable model surfaces a clean `Embedding` error
    // (no panic). Model weights are unreachable in this sandbox (HF/Xet CDN
    // blocked; see mod.rs test notes), so this verifies the error path.
    #[tokio::test]
    async fn test_embed_all_no_model_errors() {
        let (_dir, qa, _mock, _router) = temp_qa().await;
        qa.import_document(Path::new("/docs/wf.md"), "wf.md", ZH_DOC)
            .unwrap();
        let result = qa.embed_all().await;
        assert!(
            matches!(
                result,
                Err(crate::knowledge::error::KnowledgeError::Embedding(_))
            ),
            "embed_all must fail cleanly when the model cannot load"
        );
    }

    // P24: explicit model preference routes through the ModelRouter and is
    // reflected in `meta.engine`. The router builds a REAL provider from config
    // (a network call), so this needs a reachable endpoint + key → `#[ignore]`.
    #[tokio::test]
    #[ignore]
    async fn test_ask_model_parameter_routes() {
        let (_dir, qa, _mock, _router) = temp_qa().await;
        qa.import_document(Path::new("/docs/wf.md"), "wf.md", ZH_DOC)
            .unwrap();
        // Task-specified model "deepseek-chat" resolves through the router.
        let resp = qa
            .ask(
                "工作流引擎是怎么实现的？",
                5,
                RetrievalStrategy::Keyword,
                Some("deepseek-chat"),
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(resp.meta.engine, "deepseek-chat");
        assert!(resp.meta.reason.starts_with("model: deepseek-chat"));
        assert!(!resp.answer.is_empty());
    }

    // P24: when the router can resolve no model (the requested model and its
    // entire fallback chain are all unhealthy), `ask` surfaces a clean
    // ModelSelection error rather than panicking or making a doomed LLM call.
    #[tokio::test]
    async fn test_ask_model_selection_error_path() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("knowledge.db");
        let store = Arc::new(SqliteKnowledgeStore::open(&db).unwrap());
        // The held `provider` is irrelevant here: a model preference is supplied,
        // so `ask` routes through the ModelRouter (which builds a real provider
        // from config) — and the router fails *before* any LLM call.
        let provider = Arc::new(MockLlmProvider::single("x".to_string()));
        let cfg = Arc::new(
            crate::config::ConfigManager::init_with_paths(Some(dir.path()))
                .await
                .unwrap(),
        );
        // Mark the requested model AND its only fallback ("gpt-4o-mini") unhealthy.
        // The default sample config wires deepseek-chat → fallback gpt-4o-mini, so
        // the whole priority+fallback chain is now dead.
        let health = Arc::new(crate::router::health::InMemoryHealthChecker::new());
        health.mark_unhealthy("deepseek-chat");
        health.mark_unhealthy("gpt-4o-mini");
        let router = Arc::new(ModelRouter::new(cfg, health));
        let qa = SqliteKnowledgeQA::new(
            store,
            provider,
            Arc::new(EmbeddingService::new(make_embedding_config())),
            router,
        );
        qa.import_document(Path::new("/docs/wf.md"), "wf.md", ZH_DOC)
            .unwrap();
        let result = qa
            .ask(
                "工作流引擎是怎么实现的？",
                5,
                RetrievalStrategy::Keyword,
                Some("deepseek-chat"),
                None,
                None,
            )
            .await;
        assert!(
            matches!(
                result,
                Err(crate::knowledge::error::KnowledgeError::ModelSelection(_))
            ),
            "expected ModelSelection error when the requested model and its fallback are all unhealthy"
        );
    }

    // ── Ignored: require a real embedding model (MiniLM ONNX staged via
    // ModelScope, per mod.rs test notes). Run with:
    //   HF_HUB_CACHE=<cache> HF_HOME=<cache> cargo test --lib -- --ignored
    // ──────────────────────────────────────────────────────────────────────

    // Acceptance #8 (semantic branch): ask(Semantic) returns the semantic path.
    #[tokio::test]
    #[ignore]
    async fn test_ask_semantic_strategy() {
        let (_dir, qa, _mock, _router) = temp_qa().await;
        qa.import_document(Path::new("/docs/wf.md"), "wf.md", ZH_DOC)
            .unwrap();
        // Real vectors require a loaded model; embed_all does that.
        let n = qa.embed_all().await.unwrap();
        assert!(n > 0);
        let resp = qa
            .ask(
                "工作流引擎是怎么实现的？",
                5,
                RetrievalStrategy::Semantic,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(resp.meta.engine, "semantic");
        assert!(!resp.answer.is_empty());
    }

    // Acceptance #9 (semantic branch): ask(Auto) with embeddings uses semantic.
    #[tokio::test]
    #[ignore]
    async fn test_ask_auto_semantic_path() {
        let (_dir, qa, _mock, _router) = temp_qa().await;
        qa.import_document(Path::new("/docs/wf.md"), "wf.md", ZH_DOC)
            .unwrap();
        qa.embed_all().await.unwrap();
        let resp = qa
            .ask(
                "工作流引擎是怎么实现的？",
                5,
                RetrievalStrategy::Auto,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(resp.meta.engine, "semantic");
    }

    // Acceptance #3/#4 (QA wrapper): semantic search by free text returns ranked
    // results (needs a loaded model to embed the query).
    #[tokio::test]
    #[ignore]
    async fn test_qa_search_semantic_returns() {
        let (_dir, qa, _mock, _router) = temp_qa().await;
        qa.import_document(Path::new("/docs/wf.md"), "wf.md", ZH_DOC)
            .unwrap();
        qa.embed_all().await.unwrap();
        let top = qa.search_semantic("工作流引擎", 5).await.unwrap();
        assert!(!top.is_empty());
        assert_eq!(top[0].document_name, "wf.md");
    }
}
