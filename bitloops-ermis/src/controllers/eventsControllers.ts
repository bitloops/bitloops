import { RouteHandlerMethod } from 'fastify';
import { CORS, UNAUTHORIZED_REQUEST } from '../constants';
import { EventRequest } from '../routes/definitions';
import { getErmisConnectionTopic } from '../helpers/topics';
import { endConnection } from '../helpers/sse';

// TODO change server to http2 for >6 connections
export const establishSseConnection: RouteHandlerMethod = async function (request: EventRequest, reply) {
	const { connectionId } = request.params;
	console.log('establishSseConnection', connectionId);

	// saves connection
	const creds = request.verification ?? UNAUTHORIZED_REQUEST.verification;
	this.services.sseConnectionsCache.cache(connectionId, reply.raw, creds);

	// subscribe to ermis connection topic
	const connectionTopic = getErmisConnectionTopic(connectionId);
	this.subscriptionEvents.subscribe(connectionTopic, this.subscriptionEvents.connectionTopicSubscribeHandler(this.services, connectionId));

	let headers = {
		'Content-Type': 'text/event-stream',
		Connection: 'keep-alive',
		'Cache-Control': 'no-cache',
		[CORS.HEADERS.ACCESS_CONTROL_ALLOW_ORIGIN]: CORS.ALLOW_ORIGIN,
	};
	reply.raw.writeHead(200, headers);
	// Very important line
	reply.raw.flushHeaders(); // TODO check if this is needed
	headers = null;

	console.log('sending reply');
	reply.sent = true;
	console.log('reply sent');
	reply.raw.write('OK');
	request.socket.on('close', () => {
		console.log('sse connection closed for', connectionId);
		endConnection(this.services, connectionId);
	});
};