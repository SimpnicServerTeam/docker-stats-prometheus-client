use actix_web::{HttpResponse, Responder, Scope, get, http::header::ContentType, web::{self, Data}};
use prometheus_client::{encoding::text, registry::Registry};

use crate::{docker_stat_metrics::DockerStatContainerMetrics, usecases};


#[derive(Debug)]
pub struct SharedAppData {
    pub host: String,
}

#[get("/health")]
async fn health() -> impl Responder {
    HttpResponse::Ok()
}

#[get("/docker/stats")]
async fn get_docker_stats(app: Data<SharedAppData>) -> HttpResponse {
    let host = &app.as_ref().host;
    match usecases::docker_stat(host).await {
        Ok(v) => HttpResponse::Ok()
            .content_type(ContentType::json())
            .body(serde_json::to_string_pretty(&v).unwrap()),
        Err(e) => HttpResponse::InternalServerError()
            .content_type(ContentType::plaintext())
            .body(e.to_string()),
    }
}

#[get("/metrics")]
async fn get_metrics(app: Data<SharedAppData>) -> HttpResponse {
    let host = &app.as_ref().host;
    match usecases::docker_stat(host).await {
        Ok(v) => {
            let mut registry = Registry::with_prefix("container");

            for stat in v {
                let metrics = DockerStatContainerMetrics::new(&stat.id);
                metrics.cpu_usage.set(stat.cpu_usage);
                metrics.mem_usage.set(stat.mem_usage);
                metrics.mem_limit.set(stat.mem_limit);
                metrics.net_in.set(stat.net_in);
                metrics.net_out.set(stat.net_out);
                metrics.blk_in.set(stat.blk_in);
                metrics.blk_out.set(stat.blk_out);
                metrics.register_as_sub_registry(&mut registry, &stat.name);
            }

            let mut body = String::new();
            match text::encode(&mut body, &registry) {
                Ok(_) => return HttpResponse::Ok()
                    .content_type("application/openmetrics-text; version=1.0.0; charset=utf-8")
                    .body(body),
                Err(e) => return HttpResponse::InternalServerError()
                    .content_type(ContentType::plaintext()).body(e.to_string()),
            }
        },
        Err(e) => return HttpResponse::InternalServerError()
            .content_type(ContentType::plaintext()).body(e.to_string()),
    }
}

pub fn get_scopes(path: &str) -> Scope {
    web::scope(path)
        .service(health)
        .service(get_docker_stats)
        .service(get_metrics)
}