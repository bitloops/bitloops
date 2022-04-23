import { JSONCodec, MsgHdrs } from 'nats';
import { AppOptions } from './constants';
import { Options } from './services';
import { IMQ } from './services/interfaces';

type BitloopsEvent = {
	workspaceId: string;
	messageId: string;
	payload: unknown;
	context?: unknown;
};
type BitloopsRequest = {
	workspaceId: string;
	workflowId: string;
	workflowVersion?: string;
	environmentId: string;
	payload: unknown;
	context?: unknown;
};
export type BitloopsEventResponse = {
	contentType: string;
	content: string;
	error: Error;
};

export const bitloopsRequestResponse = async (
	bitloopsRequestArgs: BitloopsRequest,
	mq: IMQ,
	headers?: MsgHdrs,
): Promise<any> => {
	const { workspaceId, workflowId, payload } = bitloopsRequestArgs;
	console.log(`Sending...`, workspaceId, workflowId, payload);
	const natsConnection = await mq.getConnection();

	const m = await natsConnection.request(
		Options.getOption(AppOptions.BITLOOPS_ENGINE_EVENTS_TOPIC) ?? 'test-bitloops-engine-events',
		JSONCodec<BitloopsRequest>().encode(bitloopsRequestArgs),
		{ timeout: 10000, headers },
	);
	const response = JSONCodec().decode(m.data);
	// console.log('got response:', response);
	return response;
};

export const bitloopsMessage = async (bitloopsPublishArgs: BitloopsEvent, mq: IMQ): Promise<void> => {
	const natsConnection = await mq.getConnection();
	return natsConnection.publish(
		Options.getOption(AppOptions.BITLOOPS_ENGINE_EVENTS_TOPIC) ?? 'test-bitloops-engine-events',
		JSONCodec<BitloopsEvent>().encode(bitloopsPublishArgs),
	);
};
