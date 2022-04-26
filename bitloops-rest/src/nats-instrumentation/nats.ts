import {
	InstrumentationBase,
	InstrumentationConfig,
	InstrumentationModuleDefinition,
	InstrumentationNodeModuleDefinition,
} from '@opentelemetry/instrumentation';
import { context, diag, propagation, ROOT_CONTEXT, trace } from '@opentelemetry/api';
import * as nats from 'nats';
import * as natsProtocol from 'nats/lib/nats-base-client/protocol';
import { SubscriptionImpl } from 'nats/lib/nats-base-client/subscription';
import { Request } from 'nats/lib/nats-base-client/request';
import { PublishOptions, MsgHdrs, RequestOptions } from 'nats';
import PropagatorUtils from './propagator-utils';
import {
	MessagingDestinationKindValues,
	MessagingOperationValues,
	SemanticAttributes,
} from '@opentelemetry/semantic-conventions';
import { Context, SpanKind } from '@opentelemetry/api';

export default class NatsInstrumentation extends InstrumentationBase<typeof nats> {
	static readonly component = 'nats';

	static natsHeaders: () => MsgHdrs;

	constructor(config?: InstrumentationConfig) {
		super('opentelemetry-nats-instrumentation', '0.0.1', config);
	}

	protected init(): InstrumentationModuleDefinition<typeof nats | typeof natsProtocol>[] {
		const modules = [
			new InstrumentationNodeModuleDefinition<typeof nats>(
				NatsInstrumentation.component,
				['*'],
				this.patchNats.bind(this),
				this.unPatchNats.bind(this),
			),
			new InstrumentationNodeModuleDefinition<typeof natsProtocol>(
				'nats/lib/nats-base-client/protocol',
				['*'],
				this.patchNatsProtocol.bind(this),
				this.unPatchNatsProtocol.bind(this),
			),
		];
		return modules;
	}

	protected patchNats(moduleExports: typeof nats, moduleVersion: string) {
		diag.debug(`@opentelemetry Applying patch for nats@${moduleVersion}`);
		NatsInstrumentation.natsHeaders = moduleExports.headers;
		return moduleExports;
	}

	protected unPatchNats(moduleExports: typeof nats) {
		diag.debug('nats instrumentation: un-patching');
	}

	protected patchNatsProtocol(moduleExports: typeof natsProtocol, moduleVersion: string) {
		diag.debug(`@opentelemetry Applying patch for nats/lib/nats-base-client/protocol@${moduleVersion}`);
		const self = this;
		// console.log(`subscribe :${moduleVersion}`, moduleExports.ProtocolHandler.prototype.subscribe);
		// console.log(`publish :${moduleVersion}`, moduleExports.ProtocolHandler.prototype.publish);

		this._wrap(moduleExports.ProtocolHandler.prototype, 'subscribe', self.getSubscribePatch.bind(self));
		this._wrap(moduleExports.ProtocolHandler.prototype, 'publish', self.getPublishPatch.bind(self));
		return moduleExports;
	}

	protected unPatchNatsProtocol(moduleExports) {
		diag.debug('nats/protocol instrumentation: un-patching');
	}

	private getSubscribePatch(original: (...args: unknown[]) => Promise<void>) {
		const self = this;
		diag.debug('Wrapping subscribe method');

		return function (s: SubscriptionImpl) {
			const originalCallback = s.callback;
			s.callback = self.subscriberCallbackWrapper(originalCallback);
			const result = original.apply(this, [s]);
			return result;
		};
	}

	private getPublishPatch(original: (...args: unknown[]) => Promise<void>) {
		const self = this;
		diag.debug('Wrapping publish method');

		return function (subject: string, data: Uint8Array, options?: PublishOptions) {
			options = options ?? {};
			const activeSpan = self.startPublisherSpan(subject, options);
			const result = original.apply(this, [subject, data, options]);
			activeSpan.end();
			return result;
		};
	}

	private subscriberCallbackWrapper(originalCallback) {
		const self = this;
		return function (err: nats.NatsError, msg: nats.Msg) {
			const propagatedContext = propagation.extract(ROOT_CONTEXT, msg.headers, PropagatorUtils.getter);
			const span = self.startSubscriberSpan(msg.subject, MessagingOperationValues.PROCESS, propagatedContext);
			// TODO understand context.with
			const cbPromise = context.with(trace.setSpan(propagatedContext, span), () => {
				return originalCallback(err, msg);
			});
			Promise.resolve(cbPromise).finally(() => {
				span.end();
			});
		};
	}

	private startSubscriberSpan(topic: string, operation: string, context: Context) {
		const span = this.tracer.startSpan(
			`${topic} ${operation}`,
			{
				kind: SpanKind.CONSUMER,
				attributes: {
					[SemanticAttributes.MESSAGING_SYSTEM]: 'nats',
					[SemanticAttributes.MESSAGING_DESTINATION]: topic,
					[SemanticAttributes.MESSAGING_DESTINATION_KIND]: MessagingDestinationKindValues.TOPIC,
					[SemanticAttributes.MESSAGING_OPERATION]: operation,
				},
			},
			context,
		);
		return span;
	}

	private startPublisherSpan(topic: string, options: PublishOptions) {
		// const activeSpan = trace.getSpan(context.active());
		const span = this.tracer.startSpan(`${topic} send`, {
			kind: SpanKind.PRODUCER,
			attributes: {
				[SemanticAttributes.MESSAGING_SYSTEM]: 'nats',
				[SemanticAttributes.MESSAGING_DESTINATION]: topic,
				[SemanticAttributes.MESSAGING_DESTINATION_KIND]: MessagingDestinationKindValues.TOPIC,
			},
		});

		options.headers = options.headers ?? NatsInstrumentation.natsHeaders();
		propagation.inject(trace.setSpan(context.active(), span), options.headers, PropagatorUtils.setter);
		return span;
	}
}
