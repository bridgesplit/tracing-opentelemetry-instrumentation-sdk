use opentelemetry::trace::TraceError;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{LogExporterBuilder, TonicExporterBuilder};
use opentelemetry_sdk::{
    logs::{BatchLogProcessor, LoggerProvider},
    runtime,
    trace::Tracer,
};
use tracing::{info, Subscriber};
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::{filter::EnvFilter, fmt::{self, writer::MakeWriterExt}, layer::SubscriberExt, registry::LookupSpan, Layer};

use crate::Error;

#[cfg(not(feature = "logfmt"))]
#[must_use]
pub fn build_logger_text<S>() -> Box<dyn Layer<S> + Send + Sync + 'static>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    use tracing_subscriber::fmt::format::FmtSpan;
    if cfg!(debug_assertions) {
        Box::new(
            tracing_subscriber::fmt::layer()
                .pretty()
                .with_line_number(true)
                .with_thread_names(true)
                .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
                .with_timer(tracing_subscriber::fmt::time::uptime()),
        )
    } else {
        Box::new(
            tracing_subscriber::fmt::layer()
                .json()
                //.with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
                .with_timer(tracing_subscriber::fmt::time::uptime()),
        )
    }
}

#[cfg(feature = "logfmt")]
#[must_use]
pub fn build_logger_text<S>() -> Box<dyn Layer<S> + Send + Sync + 'static>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    Box::new(tracing_logfmt_otel::layer())
}

#[must_use]
pub fn build_loglevel_filter_layer() -> tracing_subscriber::filter::EnvFilter {
    // filter what is output on log (fmt)
    // std::env::set_var("RUST_LOG", "warn,otel::tracing=info,otel=debug");
    std::env::set_var(
        "RUST_LOG",
        format!(
            // `otel::tracing` should be a level info to emit opentelemetry trace & span
            // `otel::setup` set to debug to log detected resources, configuration read and infered
            "{},otel::tracing=trace,otel=debug",
            std::env::var("RUST_LOG")
                .or_else(|_| std::env::var("OTEL_LOG_LEVEL"))
                .unwrap_or_else(|_| "info".to_string())
        ),
    );
    EnvFilter::from_default_env()
}

pub fn build_otel_layer<S>() -> Result<OpenTelemetryLayer<S, Tracer>, TraceError>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    use crate::{
        init_propagator, //stdio,
        otlp,
        resource::DetectResource,
    };
    let otel_rsrc = DetectResource::default()
        //.with_fallback_service_name(env!("CARGO_PKG_NAME"))
        //.with_fallback_service_version(env!("CARGO_PKG_VERSION"))
        .build();
    let otel_tracer = otlp::init_tracer(otel_rsrc, otlp::identity)?;
    // to not send trace somewhere, but continue to create and propagate,...
    // then send them to `axum_tracing_opentelemetry::stdio::WriteNoWhere::default()`
    // or to `std::io::stdout()` to print
    //
    // let otel_tracer = stdio::init_tracer(
    //     otel_rsrc,
    //     stdio::identity::<stdio::WriteNoWhere>,
    //     stdio::WriteNoWhere::default(),
    // )?;
    init_propagator()?;
    Ok(tracing_opentelemetry::layer()
        .with_error_records_to_exceptions(true)
        .with_tracer(otel_tracer))
}

pub fn init_subscribers() -> Result<(), Error> {
    //setup a temporary subscriber to log output during setup
    let subscriber = tracing_subscriber::registry()
        .with(build_loglevel_filter_layer())
        .with(build_logger_text());
    let _guard = tracing::subscriber::set_default(subscriber);
    info!("init logging & tracing");

    //Create an exporter that writes to stdout
    let exporter = LogExporterBuilder::Tonic(TonicExporterBuilder::default())
        .build_log_exporter()
        .unwrap();
    //Create a LoggerProvider and register the exporter
    let logger_provider = LoggerProvider::builder()
        .with_log_processor(BatchLogProcessor::builder(exporter, runtime::Tokio).build())
        .build();

        let logfile = tracing_appender::rolling::hourly("/logging", "app.log");
        // Log `INFO` and above to stdout.
        let stdout = std::io::stdout.with_max_level(tracing::Level::INFO);

    let subscriber = tracing_subscriber::registry()
        .with(fmt::Layer::new().with_writer(stdout.and(logfile)).with_ansi(false))
        .with(build_otel_layer()?)
        .with(OpenTelemetryTracingBridge::new(&logger_provider))
        .with(build_loglevel_filter_layer())
        .with(build_logger_text());
    tracing::subscriber::set_global_default(subscriber)?;
    Ok(())
}
