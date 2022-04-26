import { RouteHandlerMethod } from 'fastify';
import { CORS } from '../constants';
import {
	SubscribeRequest,
	UnSubscribeRequest,
} from '../routes/definitions';
import { getErmisConnectionIdTopic } from '../helpers/topics';

const SUBSCRIBE_ACTION = 'subscribe';
const UNSUBSCRIBE_ACTION = 'unsubscribe';
// TODO change server to http2 for >6 connections

const headers = { [CORS.HEADERS.ACCESS_CONTROL_ALLOW_ORIGIN]: CORS.ALLOW_ORIGIN };

export const subscribeHandler: RouteHandlerMethod = async function (request: SubscribeRequest, reply) {
	const { topic, workspaceId } = request.body;
	let { connectionId } = request.params;

	console.log('subscribeHandler connectionId', connectionId);

	const ermisPayload = {
		topic,
		workspaceId,
		action: SUBSCRIBE_ACTION
	};

	const ermisTopic = getErmisConnectionIdTopic(connectionId);
	console.log('subscribeHandler ermisPayload', ermisPayload);

	await this.mq.publish(ermisTopic, ermisPayload);
	reply.status(201).headers(headers).send();
};

export const unsubscribeHandler: RouteHandlerMethod = async function (request: UnSubscribeRequest, reply) {
	let { connectionId } = request.params;
	const { workspaceId, topic } = request.body;
	console.log('unsubscribe handler', connectionId);

	const ermisPayload = {
		topic,
		workspaceId,
		action: UNSUBSCRIBE_ACTION
	};

	const ermisTopic = getErmisConnectionIdTopic(connectionId);

	await this.mq.publish(ermisTopic, ermisPayload);
	reply.status(201).headers(headers).send();
};
