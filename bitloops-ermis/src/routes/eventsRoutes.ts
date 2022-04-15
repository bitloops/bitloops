import { IMQ } from '../services/MQ/interfaces';
import { FastifyInstance } from 'fastify';
import { authMiddleware } from './helpers';
import { Services as TServices } from '../services/definitions';

import * as EventsController from '../controllers/eventsControllers';

const injectedEventsRoutes = (services: TServices, subscriptionEvents) => async (fastify: FastifyInstance, _options) => {
	fastify.decorate('services', services);
	fastify.decorate('subscriptionEvents', subscriptionEvents);
	fastify
		.get(
			'/:connectionId',
			{
				// schema: { params: EventsParams, headers: AuthHeadersSchema }, // TODO see how we can add validations again but allow for unauthorized subscriptions
				preHandler: authMiddleware,
			},
			EventsController.establishSseConnection,
		)
};

export default injectedEventsRoutes;
