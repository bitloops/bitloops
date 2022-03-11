import { FastifyInstance } from 'fastify';
import { TwilioEventRequest } from './definitions';

type TwilioRequest = {
	Params: TwilioEventRequest;
	Querystring: any;
};
const twilioRoutes = async (fastify: FastifyInstance, _opts, done) => {
	fastify.get<TwilioRequest>('/:event', async (request, reply) => {
		const { params, query, headers } = request; //
		// const { mq } = Services.getServices();
		// console.log('headers');
		// console.log(request.headers);
		// console.log('parameters');
		// console.log(request.params);
		// if (headers?.authorization) {
		// 	const { token } = extractAuthTypeAndToken(headers?.authorization);
		// 	const authInfo = await getCredentialInfo(`credentials:Basic:${token}`, redisClient);
		// 	console.log(authInfo.workspaceId, authInfo.env, authInfo.error);
		// 	if (authInfo.error)
		// 		reply
		// 			.code(401)
		// 			.type('text/xml')
		// 			.header('WWW-Authenticate', 'Basic realm="Bitloops API"')
		// 			.send(`<?xml version="1.0" encoding="UTF-8"?><Error>${authInfo.error}</Error>`);
		// 	else {
		// 		console.log(query);
		// 		console.log(params);
		// 		// console.log('body');
		// 		// console.log(request.body);
		// const messageId = `twilio:${params.event}`;
		// 		const inputs = { ...query, ...params };
		// 		console.log(authInfo.workspaceId, messageId);
		// 		const rep = await bitloopsRequestResponse(authInfo.workspaceId, messageId, inputs, null, mq);
		// 		// const rep = { contentType: 'text/plain', content: 'OK' };
		// 		console.log('NATS reply');
		// 		console.log(rep);
		// 		reply.code(200).header('Content-Type', rep.contentType).send(rep.content);
		// 	}
		// } else {
		// 	reply
		// 		.code(401)
		// 		.type('text/xml')
		// 		.header('WWW-Authenticate', 'Basic realm="Bitloops API"')
		// 		.send(`<?xml version="1.0" encoding="UTF-8"?><Error>401 Unauthorized</Error>`);
		// }
	});
};

export default twilioRoutes;
