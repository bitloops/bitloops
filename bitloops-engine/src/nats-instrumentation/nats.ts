import {
	InstrumentationBase,
	InstrumentationConfig,
	InstrumentationModuleDefinition,
	InstrumentationNodeModuleDefinition,
} from '@opentelemetry/instrumentation';
import * as api from '@opentelemetry/api';
import * as nats from 'nats';
// import { ProtocolHandler } from 'nats/lib/nats-base-client/protocol';
import * as natsProtocol from 'nats/lib/nats-base-client/protocol';
// import * as natsProtocol from 'nats/lib/nats-base-client/headers';
import { SubscriptionImpl } from 'nats/lib/nats-base-client/subscription';
import { PublishOptions, MsgHdrs } from 'nats';
import ParseUtils from './parse-utils';
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
		super('my-nats-instrumentation', '0.0.1', config);
	}

	protected init(): InstrumentationModuleDefinition<typeof nats | typeof natsProtocol>[] {
		console.log('nats init');

		return [
			new InstrumentationNodeModuleDefinition(
				NatsInstrumentation.component,
				['*'], // any version of nats
				this.patchNats.bind(this),
				this.unPatchNats.bind(this),
			),
			new InstrumentationNodeModuleDefinition(
				'nats/lib/nats-base-client/protocol',
				['*'], // any version of nats
				// invoked every time that nats module is loaded/patched
				// invoked on un-patching of module
				this.patchNatsProtocol.bind(this),
				(moduleExports) => {},
			),
		];
	}

	protected patchNats(moduleExports: typeof nats, moduleVersion: string) {
		api.diag.debug('nats instrumentation: patching');
		// we could differ this callback based of nats moduleVersion
		const self = this;
		console.log(`nats version: ${moduleVersion}`);
		NatsInstrumentation.natsHeaders = moduleExports.headers;
		// We need to return the module
		return moduleExports;
	}

	protected unPatchNats(moduleExports) {
		api.diag.debug('nats instrumentation: un-patching');
	}

	protected patchNatsProtocol(moduleExports: any, moduleVersion) {
		// we could differ this callback based of nats moduleVersion
		// moduleExports.;
		const self = this;
		console.log(`nats/lib/nats-base-client/protocol version: ${moduleVersion}`);

		console.log(`subscribe :${moduleVersion}`, moduleExports.ProtocolHandler.prototype.subscribe);
		console.log(`publish :${moduleVersion}`, moduleExports.ProtocolHandler.prototype.publish);

		this._wrap(moduleExports.ProtocolHandler.prototype, 'subscribe', self.getSubscribePatch.bind(self));

		/**
		 * This includes publishes back to originalReply topic
		 */
		this._wrap(moduleExports.ProtocolHandler.prototype, 'publish', self.getPublishPatch.bind(this));
		// request(r: Request): Request;
		return moduleExports;
	}

	protected unPatchNatsProtocol(moduleExports) {
		api.diag.debug('nats-protocol instrumentation: un-patching');
	}

	private getSubscribePatch(original: (...args: unknown[]) => Promise<void>) {
		const self = this;

		console.log('Wrapping subscribe method');

		return function (s: SubscriptionImpl) {
			// console.log('wrapping ORIGINAL subscribe', s.callback.toString());
			const originalCb = s.callback;
			s.callback = (err: nats.NatsError, msg: nats.Msg) => {
				console.log('hi cb running :)');

				const propagatedContext = api.propagation.extract(api.ROOT_CONTEXT, msg.headers, ParseUtils.getter);
				const span = self.startSubscriberSpan(msg.subject, MessagingOperationValues.PROCESS, propagatedContext);
				// TODO understand api.context.with
				const cbPromise = api.context.with(api.trace.setSpan(propagatedContext, span), () => {
					return originalCb(err, msg);
				});
				Promise.resolve(cbPromise).finally(() => {
					span.end();
				});
			};
			const result = original.apply(this, [s]);

			return result;
		};
	}

	private getPublishPatch(original: (...args: unknown[]) => Promise<void>) {
		const self = this;
		console.log(`Wrapping publish method`);
		console.log('nats headers', NatsInstrumentation.natsHeaders);
		// console.log(moduleExports);

		return function (subject: string, data: Uint8Array, options?: PublishOptions) {
			options = options ?? {};
			const activeSpan = self.startPublisherSpan(subject, options);
			console.log('PUBLISHING TO', subject);
			const result = original.apply(this, [subject, data, options]);
			activeSpan.end();
			return result;
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
		// const activeSpan = api.trace.getSpan(api.context.active());
		const span = this.tracer.startSpan(`${topic} send`, {
			kind: SpanKind.PRODUCER,
			attributes: {
				[SemanticAttributes.MESSAGING_SYSTEM]: 'nats',
				[SemanticAttributes.MESSAGING_DESTINATION]: topic,
				[SemanticAttributes.MESSAGING_DESTINATION_KIND]: MessagingDestinationKindValues.TOPIC,
			},
		});

		options.headers = options.headers ?? NatsInstrumentation.natsHeaders();
		api.propagation.inject(api.trace.setSpan(api.context.active(), span), options.headers, ParseUtils.setter);
		return span;
	}
}
