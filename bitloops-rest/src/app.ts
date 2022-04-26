/**
 * All plugins and routes are
 * initialized here
 */
import fastify, { FastifyReply, FastifyServerOptions } from 'fastify';
import formBodyPlugin from 'fastify-formbody';
import api from '@opentelemetry/api';
import fastifyStaticPlugin from 'fastify-static';
import * as path from 'path';
import cookie from 'fastify-cookie';

import Services, { Options } from './services';
import {
	healthRoutes,
	bitloopsRoutes,
	readyRoutes,
	eventsRoutes,
	twilioRoutes,
	zendeskRoutes,
	authRoutes,
} from './routes';
import { CORS, MQTopics } from './constants';
import SubscriptionEvents from './handlers/SubscriptionEvents/SubscriptionEvents';
import { requestEventRequest } from './routes/definitions';
import { sseConnectionIds } from './controllers/events.controllers';

const optionsHandler = async (request: requestEventRequest, reply: FastifyReply) => {
	reply
		.code(200)
		.headers({
			[CORS.HEADERS.ACCESS_CONTROL_ALLOW_ORIGIN]: CORS.ALLOW_ORIGIN,
			[CORS.HEADERS.ACCESS_CONTROL_ALLOW_HEADERS]: CORS.ALLOW_HEADERS,
		})
		.send('OK');
};

const build = async (opts: FastifyServerOptions = {}) => {
	const server = fastify(opts);
	const services = await Services.initializeServices();
	const { mq, imdb, db } = services;

	server
		// Needed for Open-Telemetry
		.register(require('fastify-express'))
		.register(formBodyPlugin)
		.register(fastifyStaticPlugin, {
			root: path.join(__dirname, 'public'),
		})
		.options('*', optionsHandler)
		.register(healthRoutes, { prefix: '/healthy' })
		.register(readyRoutes, { prefix: '/ready' })
		.register(bitloopsRoutes, { prefix: '/bitloops' })
		.register(twilioRoutes, { prefix: '/twilio' })
		.register(zendeskRoutes, { prefix: '/zendesk' })
		.register(eventsRoutes(mq), { prefix: '/bitloops/events' })
		.register(cookie)
		.register(authRoutes(mq, imdb, db), { prefix: '/bitloops/auth' })
		.setNotFoundHandler((_req, reply) => {
			reply.status(404).send('Route not found');
		});

	/** NATS subscriptions */
	const subscriptionEvents = new SubscriptionEvents(sseConnectionIds);

	const serverId = Options.getServerUUID();
	const workflowEventsTopic = `${serverId}.${Options.getOption(MQTopics.WORKFLOW_EVENTS_TOPIC)}.>`;
	mq.subscribe(workflowEventsTopic, (data, subject) => subscriptionEvents.handle(data, subject));
	console.log(`REST listening to ${workflowEventsTopic}`);
	return server;
};

export { build };
