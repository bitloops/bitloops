import opentelemetry, { DiagLogLevel, DiagConsoleLogger, diag } from '@opentelemetry/api';
import { JaegerExporter } from '@opentelemetry/exporter-jaeger';
// diag.setLogger(new DiagConsoleLogger(), +process.env.OPEN_TELEMETRY_LOG ?? DiagLogLevel.NONE);

import { Resource } from '@opentelemetry/resources';
import { SemanticResourceAttributes } from '@opentelemetry/semantic-conventions';
import { registerInstrumentations } from '@opentelemetry/instrumentation';
import { ConsoleSpanExporter, SimpleSpanProcessor, BatchSpanProcessor } from '@opentelemetry/sdk-trace-base';
import { FastifyInstrumentation } from '@opentelemetry/instrumentation-fastify';
import { HttpInstrumentation } from '@opentelemetry/instrumentation-http';
import { OTTracePropagator } from '@opentelemetry/propagator-ot-trace';
import { NodeTracerProvider } from '@opentelemetry/sdk-trace-node';
import NatsInstrumentation from './nats-instrumentation/nats';
import { PrometheusExporter } from '@opentelemetry/exporter-prometheus';
// import { MeterProvider } from '@opentelemetry/sdk-metrics-base';
import { AppOptions } from '../constants';
import { OTLPTraceExporter } from '@opentelemetry/exporter-trace-otlp-http';
import { OTLPMetricExporter } from '@opentelemetry/exporter-metrics-otlp-http';
import { MeterProvider } from '@opentelemetry/sdk-metrics-base';

export default (serviceName: string, environment: string) => {
	/**
	 * Prometheus scrapes the data from the service,
	 *  so we need to expose a port for an API endpoint
	 */
	const metricsOptions = { port: 9100 };
	const metricsExporter = new PrometheusExporter(metricsOptions, () => {
		console.log(`scrape http://localhost:${metricsOptions.port}/metrics`);
	});
	// const metricsExporter = new OTLPMetricExporter({
	// 	url: 'http://localhost:4318/v1/metrics',
	// });
	// Register the metrics-exporter
	const meter = new MeterProvider({
		exporter: metricsExporter,
		interval: 1000,
	}).getMeter(serviceName);
	/**
	 * When initializing the provider we can
	 * control/config how spans are generated
	 */
	const provider = new NodeTracerProvider({
		resource: new Resource({
			[SemanticResourceAttributes.SERVICE_NAME]: serviceName,
			[SemanticResourceAttributes.DEPLOYMENT_ENVIRONMENT]: environment,
		}),
	});

	/**
	 * What kind of libraries to be able to instrument,
	 * to collect data from
	 */
	registerInstrumentations({
		tracerProvider: provider,
		instrumentations: [
			// Fastify instrumentation expects HTTP layer to be instrumented
			new HttpInstrumentation(), // node native http library
			new FastifyInstrumentation(),
			new NatsInstrumentation(),
		],
	});

	// const exporter = new CollectorTraceExporter();
	// const traceExporter = new JaegerExporter({
	// 	endpoint: process.env[AppOptions.JAEGER_URL] ?? 'http://localhost:14268/api/traces',
	// });
	const traceExporter = new OTLPTraceExporter({
		url: 'http://localhost:4318/v1/traces',
	});
	// Generic setups
	/**
	 * By configuring the Processor we can filter out
	 * unwanted traces e.g. metrics, health checks etc.
	 */
	provider.addSpanProcessor(new SimpleSpanProcessor(traceExporter));
	// We can add a second exporter for debugging reasons
	// provider.addSpanProcessor(new BatchSpanProcessor(new ConsoleSpanExporter()));
	// provider.addSpanProcessor(new BatchSpanProcessor(traceExporter));
	// Initialize the OpenTelemetry APIs to use the NodeTracerProvider bindings
	provider.register({});
	return {
		provider,
		tracer: provider.getTracer(serviceName),
		meter,
	};
};
