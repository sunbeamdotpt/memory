// API Handlers
use actix_web::{web, HttpResponse};
use crate::api::types::{
    FactRequest, FactResponse, SearchParams, ListParams,
    SearchResponse, ErrorResponse, HealthResponse,
};
use crate::memory::service::MemoryService;
use crate::mcp::server::parse_ts;

const MAX_LIMIT: usize = 1000;

fn fact_to_response(fact: crate::memory::service::MemoryFact, score: Option<f32>) -> FactResponse {
    FactResponse {
        id: fact.id,
        namespace: fact.namespace,
        content: fact.content,
        created_at: fact.created_at,
        score: score.or(Some(fact.score)),
        source: fact.source,
    }
}

pub async fn health_check() -> HttpResponse {
    HttpResponse::Ok().json(HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

pub async fn store_fact(
    memory: web::Data<MemoryService>,
    body: web::Json<FactRequest>,
) -> HttpResponse {
    let namespace = body.namespace.as_deref().unwrap_or("default");
    match memory.add_fact(namespace, &body.content, body.source.as_deref()).await {
        Ok(fact) => HttpResponse::Created().json(fact_to_response(fact, None)),
        Err(e) => HttpResponse::InternalServerError()
            .json(ErrorResponse { error: e.to_string() }),
    }
}

pub async fn search_facts(
    memory: web::Data<MemoryService>,
    params: web::Query<SearchParams>,
) -> HttpResponse {
    if params.q.trim().is_empty() {
        return HttpResponse::BadRequest()
            .json(ErrorResponse { error: "`q` must not be empty".to_string() });
    }
    let limit = params.limit.unwrap_or(10).min(MAX_LIMIT);
    if limit == 0 {
        return HttpResponse::BadRequest()
            .json(ErrorResponse { error: "limit must be >= 1".to_string() });
    }
    match memory.search_facts(&params.q, limit, params.namespace.as_deref()).await {
        Ok(results) => {
            let total = results.len();
            HttpResponse::Ok().json(SearchResponse {
                results: results.into_iter().map(|f| fact_to_response(f, None)).collect(),
                total,
            })
        }
        Err(e) => HttpResponse::InternalServerError()
            .json(ErrorResponse { error: e.to_string() }),
    }
}

pub async fn list_facts(
    memory: web::Data<MemoryService>,
    namespace: web::Path<String>,
    params: web::Query<ListParams>,
) -> HttpResponse {
    let limit = params.limit.unwrap_or(50).min(MAX_LIMIT);
    if limit == 0 {
        return HttpResponse::BadRequest()
            .json(ErrorResponse { error: "limit must be >= 1".to_string() });
    }
    let from_ts = params.from.as_deref().and_then(parse_ts);
    let to_ts = params.to.as_deref().and_then(parse_ts);
    match memory.list_facts(&namespace, limit, from_ts, to_ts).await {
        Ok(facts) => {
            let total = facts.len();
            HttpResponse::Ok().json(SearchResponse {
                results: facts.into_iter().map(|f| fact_to_response(f, None)).collect(),
                total,
            })
        }
        Err(e) => HttpResponse::InternalServerError()
            .json(ErrorResponse { error: e.to_string() }),
    }
}

pub async fn delete_fact(
    memory: web::Data<MemoryService>,
    id: web::Path<String>,
) -> HttpResponse {
    match memory.delete_fact(&id).await {
        Ok(true)  => HttpResponse::NoContent().finish(),
        Ok(false) => HttpResponse::NotFound()
            .json(ErrorResponse { error: format!("fact {id} not found") }),
        Err(e) => HttpResponse::InternalServerError()
            .json(ErrorResponse { error: e.to_string() }),
    }
}
