import { IMQ } from '../services/MQ/interfaces';
import { RouteHandlerMethod } from 'fastify';
import { v4 as uuid } from 'uuid';
import { Options } from '../services';
import { CORS, /**MQTopics**/ } from '../constants';
import {
	EventRequest,
	// 	SrResponse,
	SubscribeRequest,
	// 	TSseConnectionIds,
	UnSubscribeRequest,
} from '../routes/definitions';

export const NULL_CONNECTION_ID = '';
// export const SR_SSE_SERVER_TOPIC = Options.getOption(MQTopics.SR_SSE_SERVER_TOPIC) ?? 'ssevent';
// TODO change server to http2 for >6 connections

const headers = { [CORS.HEADERS.ACCESS_CONTROL_ALLOW_ORIGIN]: CORS.ALLOW_ORIGIN };

const endConnection = (connectionId: string) => {
	// delete sseConnectionIds[connectionId];
};

export const establishSseConnection: RouteHandlerMethod = async function (request: EventRequest, reply) {
	const { connectionId } = request.params;
	let headers = {
		'Content-Type': 'text/event-stream',
		Connection: 'keep-alive',
		'Cache-Control': 'no-cache',
		[CORS.HEADERS.ACCESS_CONTROL_ALLOW_ORIGIN]: CORS.ALLOW_ORIGIN,
	};
	reply.raw.writeHead(200, headers);
	// Very important line
	reply.raw.flushHeaders();

	headers = null;
	// https://www.fastify.io/docs/latest/Reply/#sent
	reply.sent = true;
	request.socket.on('close', () => {
		console.log('sse connection closed for', connectionId);
		endConnection(connectionId);
	});
};

export const subscribeHandler: RouteHandlerMethod = async function (request: SubscribeRequest, reply) {
	// const topic = `${prefix}.${workspaceId}.${evalTopic.name}`;

	// const { topics, workspaceId } = request.body;
	// let { connectionId } = request.params;
	// let newConnection: boolean;
	// console.log('connectionId', connectionId);
	// if (!connectionId) {
	// 	connectionId = uuid();
	// 	newConnection = true;
	// } else {
	// 	newConnection = false;
	// 	const res = await this.mq.request<SrResponse>(`${SR_SSE_SERVER_TOPIC}.${connectionId}`, {
	// 		type: { name: SSE_MESSAGE_TYPE.VALIDATION },
	// 	});
	// 	if (!res.result) {
	// 		reply.status(404).headers(headers).send('ConnectionId not found');
	// 		return;
	// 	}
	// }
	// // Inform Subscribe Router for new subscriptions
	// const payload = {
	// 	type: {
	// 		name: SSE_MESSAGE_TYPE.TOPICS_ADD_CONNECTION,
	// 		newConnection,
	// 		workspaceId,
	// 		topics,
	// 	},
	// };
	// if (newConnection) payload.type['creds'] = request.verification ? request.verification : 'Unauthorized';

	// // console.log('writeRes', `${SR_SSE_SERVER_TOPIC}.${connectionId}`, payload);
	// const writeRes = await this.mq.request<SrResponse>(`${SR_SSE_SERVER_TOPIC}.${connectionId}`, payload);
	// if (writeRes.error) return reply.status(500).headers(headers).send('INTERNAL SERVER ERROR');
	// if (newConnection) reply.status(201).headers(headers).send(connectionId);
	// else reply.status(204).headers(headers).send();
};

export const unsubscribeHandler: RouteHandlerMethod = async function (request: UnSubscribeRequest, reply) {
	let { connectionId } = request.params;
	const { workspaceId, topic } = request.body;
	console.log('unsubscribe handler', connectionId);
	// const res = await this.mq.request<SrResponse>(`${SR_SSE_SERVER_TOPIC}.${connectionId}`, {
	// 	type: { name: SSE_MESSAGE_TYPE.VALIDATION },
	// });
	// if (!res.result) {
	// 	reply.status(404).headers(headers).send('ConnectionId not found');
	// 	return;
	// }
	// await this.mq.request<SrResponse>(`${SR_SSE_SERVER_TOPIC}.${connectionId}`, {
	// 	type: { name: SSE_MESSAGE_TYPE.TOPIC_UNSUBSCRIBE, topic, workspaceId },
	// });

	reply.status(204).headers(headers).send();
};
