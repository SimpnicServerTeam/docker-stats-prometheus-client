use std::sync::Arc;

use actix_web::{
    HttpResponse, Responder, Scope, get,
    http::header::ContentType,
    web::{self, Data, Query},
};
use prometheus_client::encoding::text;
use serde::Deserialize;

use crate::usecases::DockerStatPollingWorker;

#[derive(Debug)]
pub struct SharedAppData {
    pub host: String,
    pub worker: Arc<DockerStatPollingWorker>,
}

#[get("/health")]
async fn health() -> impl Responder {
    HttpResponse::Ok()
}

#[get("/docker/stats")]
async fn get_docker_stats(app: Data<SharedAppData>) -> HttpResponse {
    let stats = app.worker.get_last_container_stats().await;
    HttpResponse::Ok()
        .content_type(ContentType::json())
        .body(serde_json::to_string(&stats).unwrap())
}

#[get("/metrics")]
async fn get_metrics(app: Data<SharedAppData>) -> HttpResponse {
    let registry = app.worker.get_last_container_stats_registry().await;
    let mut body = String::new();
    match text::encode(&mut body, &registry) {
        Ok(_) => {
            return HttpResponse::Ok()
                .content_type("application/openmetrics-text; version=1.0.0; charset=utf-8")
                .body(body);
        }
        Err(e) => {
            return HttpResponse::InternalServerError()
                .content_type(ContentType::plaintext())
                .body(e.to_string());
        }
    }
}

#[derive(Debug, Deserialize)]
struct GetCgroupStatsQuery {
    id: String,
}

#[get("/cgroupv2")]
async fn get_cgroup_stats(
    app: Data<SharedAppData>,
    query: Query<GetCgroupStatsQuery>,
) -> HttpResponse {
    match app.worker.get_cgroup2_data(&query.id).await {
        Ok(s) => HttpResponse::Ok()
            .content_type(ContentType::json())
            .body(serde_json::to_string(&s).unwrap()),
        Err(_) => HttpResponse::NotFound().finish(),
    }
}

pub fn get_scopes(path: &str) -> Scope {
    web::scope(path)
        .service(health)
        .service(get_docker_stats)
        .service(get_metrics)
        .service(get_cgroup_stats)
}
