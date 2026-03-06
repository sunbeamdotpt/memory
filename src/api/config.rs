use actix_web::web;
use super::handlers::{health_check, store_fact, search_facts, list_facts, delete_fact};
use super::mcp_http::{mcp_post, mcp_get, mcp_delete};

pub fn configure_api(cfg: &mut web::ServiceConfig) {
    // MCP Streamable HTTP transport endpoint
    cfg.service(
        web::resource("/mcp")
            .route(web::post().to(mcp_post))
            .route(web::get().to(mcp_get))
            .route(web::delete().to(mcp_delete))
    );

    // REST convenience API (not MCP — useful for curl / testing)
    cfg.service(
        web::scope("/api")
            .route("/health", web::get().to(health_check))
            .route("/facts", web::post().to(store_fact))
            .route("/facts/search", web::get().to(search_facts))
            .route("/facts/{namespace}", web::get().to(list_facts))
            .route("/facts/{id}", web::delete().to(delete_fact))
    );
}
