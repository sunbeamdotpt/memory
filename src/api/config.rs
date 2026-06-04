use actix_web::web;
use actix_web::middleware::from_fn;
use super::handlers::{health_check, store_fact, search_facts, list_facts, delete_fact};
use super::mcp_http::{auth_middleware, McpHttpService};

pub fn configure_api(
    mcp_service: McpHttpService,
) -> impl FnOnce(&mut web::ServiceConfig) + Clone {
    move |cfg: &mut web::ServiceConfig| {
        cfg.service(
            web::scope("/mcp")
                .wrap(from_fn(auth_middleware))
                .service(mcp_service.scope()),
        );

        cfg.service(
            web::scope("/api")
                .route("/health", web::get().to(health_check))
                .route("/facts", web::post().to(store_fact))
                .route("/facts/search", web::get().to(search_facts))
                .route("/facts/{namespace}", web::get().to(list_facts))
                .route("/facts/{id}", web::delete().to(delete_fact)),
        );
    }
}
