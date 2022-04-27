import api from '@opentelemetry/api';
import { Counter } from '@opentelemetry/api-metrics';
import { Meter } from '@opentelemetry/sdk-metrics-base';
import { Tracer } from '@opentelemetry/sdk-trace-base';
import { NodeTracerProvider } from '@opentelemetry/sdk-trace-node';
import init from './tracing';

export enum CounterName {
	RUNNING_INSTANCES = 'running_instances',
	ERROR_INSTANCES = 'error_instances',
	COMPLETED_INSTANCES = 'completed_instances',
}

export default class OpenTelemetry {
	private static _instance: OpenTelemetry;

	provider: NodeTracerProvider;

	tracer: Tracer;

	meter: Meter;

	counters: Partial<Record<CounterName, Counter>>;

	private constructor() {
		const tracing = init('bitloops-rest', 'development');
		const { provider, tracer, meter } = tracing;
		this.provider = provider;
		this.tracer = tracer;
		this.meter = meter;
		this.counters = {};
	}

	static get instance(): OpenTelemetry {
		if (!OpenTelemetry._instance) {
			OpenTelemetry.initialize();
		}
		return OpenTelemetry._instance;
	}

	/**
	 * This needs to be called before require of libraries
	 * that are instrumented. Usually it's called in the beginning
	 * of process entry file.
	 */
	static initialize() {
		OpenTelemetry._instance = new OpenTelemetry();
	}

	initializeAppCounters() {
		Object.values(CounterName).forEach((name) => {
			const counter = this.meter.createCounter(name);
			this.meter.createCounter;
			this.counters[name] = counter;
		});
	}

	increaseCounter(counterName: CounterName, value = 1) {
		const counter = this.counters[counterName];
		counter.add(value);
	}

	addTagToCurrentSpan(key: string, value: string) {
		const activeSpan = api.trace.getSpan(api.context.active());
		activeSpan.setAttribute(key, value);
	}
}
