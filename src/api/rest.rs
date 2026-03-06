use actix_web::{web, HttpResponse, Responder};

pub fn configure_rest(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/health")
            .route(web::get().to(health_check))
    );
}

async fn health_check() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({
        "status": "healthy",
        "version": env!("CARGO_PKG_VERSION")
    }))
}