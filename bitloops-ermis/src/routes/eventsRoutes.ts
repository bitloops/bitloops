import { IMQ } from '../services/MQ/interfaces';
import { FastifyInstance } from 'fastify';
import { authMiddleware } from './helpers';

import * as EventsController from '../controllers/eventsControllers';

const injectedEventsRoutes = (mq: IMQ, subscriptionEvents) => async (fastify: FastifyInstance, _options) => {
	fastify.decorate('mq', mq);
	fastify
		.get(
			'/:connectionId',
			{
				// schema: { params: EventsParams, headers: AuthHeadersSchema }, // TODO see how we can add validations again but allow for unauthorized subscriptions
				preHandler: authMiddleware,
			},
			EventsController.establishSseConnection,
		)
		.post(
			'/subscribe/:connectionId',
			{
				// schema: {
				// 	// body: PostSubscribeEventsBody,
				// 	// params: PostSubscribeEventsParams,
				// 	// headers: AuthHeadersSchema,
				// },
				preHandler: authMiddleware,
			},
			EventsController.subscribeHandler,
		)
		.post(
			'/unsubscribe/:connectionId',
			{
				preHandler: authMiddleware,
			},
			EventsController.unsubscribeHandler,
		)
		.post(
			'/test',
			(request: any, reply) => {
				const { topic } = request.body;
				subscriptionEvents.subscribe(topic);
				reply.send('OK')
			}
		);
};

export default injectedEventsRoutes;
