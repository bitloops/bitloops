import { IMQ } from '../services/interfaces/index';
import { RouteHandlerMethod } from 'fastify';
import { v4 as uuid } from 'uuid';
import { Options } from '../services';
import { CORS, MQTopics, SSE_MESSAGE_TYPE } from '../constants';
import {
	EventRequest,
	SrResponse,
	SubscribeRequest,
	TSseConnectionIds,
	UnSubscribeRequest,
} from '../routes/definitions';
import { getErmisConnectionIdTopic } from '../helpers/topics';

export const NULL_CONNECTION_ID = '';
export const SR_SSE_SERVER_TOPIC = Options.getOption(MQTopics.SR_SSE_SERVER_TOPIC) ?? 'ssevent';
const SUBSCRIBE_ACTION = 'subscribe';
const UNSUBSCRIBE_ACTION = 'unsubscribe';
// TODO change server to http2 for >6 connections
export const sseConnectionIds: TSseConnectionIds = {};

const headers = { [CORS.HEADERS.ACCESS_CONTROL_ALLOW_ORIGIN]: CORS.ALLOW_ORIGIN };

const endConnection = (mq: IMQ, connectionId: string) => {
	delete sseConnectionIds[connectionId];
	mq.publish(`${SR_SSE_SERVER_TOPIC}.${connectionId}`, {
		type: { name: SSE_MESSAGE_TYPE.CONNECTION_END },
	});
};

// TODO remove
export const establishSseConnection: RouteHandlerMethod = async function (request: EventRequest, reply) {
	const { connectionId } = request.params;
	let res = await this.mq.request<SrResponse>(`${SR_SSE_SERVER_TOPIC}.${connectionId}`, {
		type: { name: SSE_MESSAGE_TYPE.VALIDATION },
	});
	if (!res.result) {
		reply.raw.end();
		return;
	}
	let headers = {
		'Content-Type': 'text/event-stream',
		Connection: 'keep-alive',
		'Cache-Control': 'no-cache',
		[CORS.HEADERS.ACCESS_CONTROL_ALLOW_ORIGIN]: CORS.ALLOW_ORIGIN,
	};
	reply.raw.writeHead(200, headers);
	// Very important line
	reply.raw.flushHeaders();

	sseConnectionIds[connectionId] = reply.raw;
	res = await this.mq.request<SrResponse>(`${SR_SSE_SERVER_TOPIC}.${connectionId}`, {
		type: { name: SSE_MESSAGE_TYPE.POD_ID_REGISTRATION, podId: Options.getServerUUID() },
	});

	headers = null;
	res = null;
	// https://www.fastify.io/docs/latest/Reply/#sent
	reply.sent = true;
	request.socket.on('close', () => {
		console.log('sse connection closed for', connectionId);
		endConnection(this.mq, connectionId);
	});
};

export const subscribeHandler: RouteHandlerMethod = async function (request: SubscribeRequest, reply) {
	const { topic, workspaceId } = request.body;
	let { connectionId } = request.params;

	console.log('connectionId', connectionId);

	const ermisPayload = {
		topic,
		workspaceId,
		action: SUBSCRIBE_ACTION
	};

	const ermisTopic = getErmisConnectionIdTopic(connectionId);
	console.log('ermisPayload', ermisPayload);

	await this.mq.publish(ermisTopic, ermisPayload);
	reply.status(201).headers(headers).send();
};

export const unsubscribeHandler: RouteHandlerMethod = async function (request: UnSubscribeRequest, reply) {
	let { connectionId } = request.params;
	const { workspaceId, topic } = request.body;
	console.log('unsubscribe handler', connectionId);
	// TODO publish to nats topic 'ermis.{connectionId}' payload: {topic, unsubscribe, workspaceId} and remove below code

	const ermisPayload = {
		topic,
		workspaceId,
		action: UNSUBSCRIBE_ACTION
	};

	const ermisTopic = getErmisConnectionIdTopic(connectionId);

	await this.mq.publish(ermisTopic, ermisPayload);
	reply.status(201).headers(headers).send();
};
