import { IMQ } from '../services/MQ/interfaces';
import { RouteHandlerMethod } from 'fastify';
import { v4 as uuid } from 'uuid';
import { Options } from '../services';
import { CORS, UNAUTHORIZED_REQUEST, WORKFLOW_EVENTS_PREFIX, /**MQTopics**/ } from '../constants';
import {
	EventRequest,
	// 	SrResponse,
	SubscribeRequest,
	// 	TSseConnectionIds,
	UnSubscribeRequest,
} from '../routes/definitions';
import { getErmisConnectionTopic } from '../helpers/topics';
import { Services as TServices } from '../services/definitions';
import { ISubscriptionTopicsCache } from '../services/Cache/interfaces';
import { ConnectionSubscribeHandlerType } from '../handlers/interfaces';

export const NULL_CONNECTION_ID = '';
const SUBSCRIBE_ACTION = 'subscribe';
const UNSUBSCRIBE_ACTION = 'unsubscribe';
// export const SR_SSE_SERVER_TOPIC = Options.getOption(MQTopics.SR_SSE_SERVER_TOPIC) ?? 'ssevent';
// TODO change server to http2 for >6 connections

const headers = { [CORS.HEADERS.ACCESS_CONTROL_ALLOW_ORIGIN]: CORS.ALLOW_ORIGIN };

const endConnection = (services: TServices, connectionId: string) => {
	services.sseConnectionsCache.delete(connectionId);
	const topics = services.sseConnectionToTopicsCache.fetch(connectionId);
	if (topics) {
		for (let i = 0; i < topics.length; i++) {
			const topic = topics[i];
			services.sseTopicToConnectionsCache.deleteConnectionIdFromTopic(topic, connectionId);

			const connections = services.sseTopicToConnectionsCache.fetch(topic);
			if (!connections || connections.length === 0) {
				endTopic(services, topic);
			}
		}
	}
	const ermisConnectionTopic = getErmisConnectionTopic(connectionId);
	unsubscribeFromTopic(ermisConnectionTopic, services.subscriptionTopicsCache);

	services.sseConnectionToTopicsCache.delete(connectionId);
};

const endTopic = (services: TServices, topic: string) => {
	unsubscribeFromTopic(topic, services.subscriptionTopicsCache);
	const connections = services.sseTopicToConnectionsCache.fetch(topic);
	if (connections) {
		for (let i = 0; i < connections.length; i++) {
			const connectionId = connections[i];
			services.sseConnectionToTopicsCache.deleteTopicFromConnectionId(connectionId, topic);

			const topics = services.sseConnectionToTopicsCache.fetch(connectionId);
			if (!topics || topics.length === 0) {
				endConnection(services, connectionId);
			}
		}
	}
	services.sseTopicToConnectionsCache.delete(topic);
}

const unsubscribeFromTopic = (topic: string, subscriptionTopicsCache: ISubscriptionTopicsCache) => {
	const sub = subscriptionTopicsCache.fetch(topic);
	if (sub) sub.unsubscribe();
	subscriptionTopicsCache.delete(topic);
}

export const establishSseConnection: RouteHandlerMethod = async function (request: EventRequest, reply) {
	const { connectionId } = request.params;
	// TODO ask if headers are needed below
	console.log('establishSseConnection', connectionId);
	let headers = {
		'Content-Type': 'text/event-stream',
		Connection: 'keep-alive',
		'Cache-Control': 'no-cache',
		[CORS.HEADERS.ACCESS_CONTROL_ALLOW_ORIGIN]: CORS.ALLOW_ORIGIN,
	};
	reply.raw.writeHead(200, headers);
	// Very important line
	reply.raw.flushHeaders();

	// saves connection
	const creds = request.verification ?? UNAUTHORIZED_REQUEST.verification;
	this.services.sseConnectionsCache.cache(connectionId, reply.raw, creds);

	// subscribe to ermis connection topic
	const connectionTopic = getErmisConnectionTopic(connectionId);
	this.subscriptionEvents.subscribe(connectionTopic, connectionTopicSubscribeHandler(this.services, this.subscriptionEvents, connectionId));

	headers = null;
	// https://www.fastify.io/docs/latest/Reply/#sent
	reply.sent = true;
	request.socket.on('close', () => {
		console.log('sse connection closed for', connectionId);
		endConnection(this.services, connectionId);
	});
};

const connectionTopicSubscribeHandler: ConnectionSubscribeHandlerType = (services, subscriptionEvents, connectionId) => (data, subject) => {
	const { topic, workspaceId, action } = data;
	console.log('connectionTopicSubscribeHandler', subject, data);
	const finalTopic = `${WORKFLOW_EVENTS_PREFIX}.${workspaceId}.${topic}`;
	console.log('connectionTopicSubscribeHandler finalTopic', finalTopic);

	if (action === SUBSCRIBE_ACTION) {
		services.sseConnectionToTopicsCache.cache(connectionId, finalTopic);
		services.sseTopicToConnectionsCache.cache(finalTopic, connectionId);
		subscriptionEvents.subscribe(finalTopic, finalSubscribeHandler(services, topic, finalTopic));
	} else if (action === UNSUBSCRIBE_ACTION) {
		endTopic(services, finalTopic);
	}
}

//TODO maybe move to SubscriptionEvents
const notifySubscribedConnections = (services: TServices, topic: string, data: any, connections: string[]) => {
	console.log('topicConnections about to notify', connections);
	for (const connectionId of connections) {
		const { connection } = services.sseConnectionsCache.fetch(connectionId);
		if (!connection) {
			console.error('Received unexpected connection from cache');
			continue;
		}
		console.log('topic', topic);
		console.log('data', data);
		connection.write(`event: ${topic}\n`);
		connection.write(`data: ${JSON.stringify(data)}\n\n`);
	}
}

const finalSubscribeHandler = (services: TServices, topic: string, finalTopic: string) => (data, subject: string) => {
	console.log('finalSubscribeHandler topic', topic);
	console.log('finalSubscribeHandler final topic', finalTopic);

	const subscribedConnections = services.sseTopicToConnectionsCache.fetch(finalTopic);
	console.log('subscribedConnections', subscribedConnections);
	const connections = notifySubscribedConnections(services, topic, data, subscribedConnections);
}




