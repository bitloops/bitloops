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
import { AppOptions } from './constants';
import { NodeTracerProvider } from '@opentelemetry/sdk-trace-node';
import NatsInstrumentation from './nats-instrumentation/nats';

export default (serviceName: string, environment: string) => {
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
	const exporter = new JaegerExporter({
		endpoint: process.env[AppOptions.JAEGER_URL] ?? 'http://localhost:14268/api/traces',
	});
	// Generic setups
	provider.addSpanProcessor(new BatchSpanProcessor(exporter));
	// We can add a second exporter for debugging reasons
	provider.addSpanProcessor(new BatchSpanProcessor(new ConsoleSpanExporter()));
	// provider.addSpanProcessor(new BatchSpanProcessor(exporter));
	// Initialize the OpenTelemetry APIs to use the NodeTracerProvider bindings
	provider.register({});
	return {
		provider,
		tracer: provider.getTracer(serviceName),
	};
};
