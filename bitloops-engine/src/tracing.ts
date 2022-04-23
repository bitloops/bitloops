import opentelemetry, { DiagLogLevel, DiagConsoleLogger } from '@opentelemetry/api';
import { JaegerExporter } from '@opentelemetry/exporter-jaeger';

import { Resource } from '@opentelemetry/resources';
import { SemanticResourceAttributes } from '@opentelemetry/semantic-conventions';
import { registerInstrumentations } from '@opentelemetry/instrumentation';
import { ConsoleSpanExporter, SimpleSpanProcessor, BatchSpanProcessor } from '@opentelemetry/sdk-trace-base';
import { HttpInstrumentation } from '@opentelemetry/instrumentation-http';
import { OTTracePropagator } from '@opentelemetry/propagator-ot-trace';
// import { Options } from './services';
import { NodeTracerProvider } from '@opentelemetry/sdk-trace-node';
import { OPEN_TELEMETRY } from './constants';
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
			new HttpInstrumentation(), // node native http library
			new NatsInstrumentation(),
		],
	});

	// const exporter = new CollectorTraceExporter();
	const exporter = new JaegerExporter({
		endpoint: process.env[OPEN_TELEMETRY.JAEGER_ENDPOINT] ?? 'http://localhost:14268/api/traces',
	});
	// Generic setups
	provider.addSpanProcessor(new BatchSpanProcessor(exporter));
	// We can add a second exporter for debugging reasons
	// provider.addSpanProcessor(new SimpleSpanProcessor(new ConsoleSpanExporter()));
	// provider.addSpanProcessor(new BatchSpanProcessor(exporter));
	// Initialize the OpenTelemetry APIs to use the NodeTracerProvider bindings
	provider.register();
	return {
		provider,
		tracer: provider.getTracer(serviceName),
	};
};
