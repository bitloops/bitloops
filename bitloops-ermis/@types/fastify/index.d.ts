// refs https://www.typescriptlang.org/docs/handbook/declaration-merging.html#module-augmentation
import fastify from 'fastify';

declare module 'fastify' {
	export interface FastifyInstance<HttpServer = Server, HttpRequest = IncomingMessage, HttpResponse = ServerResponse> {
		services: import('../../src/services/definitions').Services;
		subscriptionEvents: import('../../src/handlers/interfaces').ISubscriptionEvents;
	}
}
