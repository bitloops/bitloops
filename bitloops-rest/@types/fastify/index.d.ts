// refs https://www.typescriptlang.org/docs/handbook/declaration-merging.html#module-augmentation
import fastify from 'fastify';

declare module 'fastify' {
	export interface FastifyInstance<HttpServer = Server, HttpRequest = IncomingMessage, HttpResponse = ServerResponse> {
		imdb: import('../../src/services/interfaces/index').IIMDB;
		mq: import('../../src/services/interfaces/index').IMQ;
		authService: import('../../src/services/Auth/interface').default;
		tracing: {
			provider: import('@opentelemetry/sdk-trace-node').NodeTracerProvider;
			tracer: import('@opentelemetry/sdk-trace-base').Tracer;
		};
	}
}
