import { FastifyInstance, FastifyReply } from 'fastify';
import { bitloopsMessage } from '../bitloops';
import { CORS } from '../constants';
import Services from '../services';
import { publishEventRequest } from './definitions';
import { getWorkspaceId, handleXApiKey, replyUnauthorized } from './helpers';

async function zendeskRoutes(fastify: FastifyInstance, _opts) {
	fastify.post('/target/comment', ticketsHandler);
}

const ticketsHandler = async (request: publishEventRequest, reply: FastifyReply) => {
	const { mq } = Services.getServices();
	const headers = request.headers;
	console.log('zendesk headers', headers);
	if (request.headers?.authorization) {
		const token = request.headers?.authorization.split(' ')[1];
		await handleXApiKey(request, reply, token);

		const messageId = `zendesk:comment`;

		const { verification } = request;

		const workspaceId = getWorkspaceId(request);

		const payload = { ...request.body, ...request.query, ...request.params, ...request.body?.payload };
		const eventArgs = {
			workspaceId,
			messageId,
			payload,
			context: verification,
		};
		await bitloopsMessage(eventArgs, mq);
		reply.code(202).headers({ [CORS.HEADERS.ACCESS_CONTROL_ALLOW_ORIGIN]: CORS.ALLOW_ORIGIN }).send('OK');
			
	} else replyUnauthorized(reply);
};

export default zendeskRoutes;
