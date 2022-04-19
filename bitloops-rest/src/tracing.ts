import opentelemetry, { DiagLogLevel, DiagConsoleLogger } from '@opentelemetry/api';
import { JaegerExporter } from '@opentelemetry/exporter-jaeger';

import { Resource } from '@opentelemetry/resources';
import { SemanticResourceAttributes } from '@opentelemetry/semantic-conventions';
import { registerInstrumentations } from '@opentelemetry/instrumentation';
import { NodeTracerProvider } from '@opentelemetry/sdk-trace-node';
import { SimpleSpanProcessor } from '@opentelemetry/sdk-trace-base';
import { FastifyInstrumentation } from '@opentelemetry/instrumentation-fastify';
import { HttpInstrumentation } from '@opentelemetry/instrumentation-http';
import { OTTracePropagator } from '@opentelemetry/propagator-ot-trace';
import { Options } from './services';
import { AppOptions } from './constants';

const jaegerOptions = {
	tags: [], // optional
	// You can use the default UDPSender
	//   host: 'localhost', // optional
	//   port: 6832, // optional
	// OR you can use the HTTPSender as follows
	endpoint: Options.getOption(AppOptions.JAEGER_URL) ?? 'http://localhost:14268/api/traces',
	//   maxPacketSize: 65000, // optional
};

export default (serviceName: string, environment: string) => {
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
		],
	});

	// const exporter = new CollectorTraceExporter();
	const exporter = new JaegerExporter(jaegerOptions);
	// Generic setups
	provider.addSpanProcessor(new SimpleSpanProcessor(exporter));
	// Initialize the OpenTelemetry APIs to use the NodeTracerProvider bindings
	provider.register({});
	return {
		provider,
		tracer: provider.getTracer(serviceName),
	};
};
