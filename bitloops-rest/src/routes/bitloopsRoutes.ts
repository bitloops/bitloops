import { CORS, PublishHeaders, RequestHeaders } from './../constants';
import { FastifyInstance, FastifyReply, RouteHandlerMethod } from 'fastify';
import { bitloopsMessage, bitloopsRequestResponse } from '../bitloops';
import Services from '../services';
import { publishEventRequest, PublishHeadersSchema, requestEventRequest, RequestHeadersSchema } from './definitions';
import { authMiddleware, getWorkspaceId } from './helpers';

async function bitloopsRoutes(fastify: FastifyInstance, _opts) {
	// TODO inject mq properly
	fastify.post('/publish', { schema: { headers: PublishHeadersSchema }, preHandler: authMiddleware }, publishHandler);
	fastify.post(
		'/request',
		{ schema: { headers: RequestHeadersSchema }, preHandler: authMiddleware },
		requestResponseHandler,
	);
}

const publishHandler: RouteHandlerMethod = async function (request: publishEventRequest, reply: FastifyReply) {
	const { mq } = Services.getServices();
	const {
		[PublishHeaders.MESSAGE_ID]: messageId,
		// [RequestHeaders.WORKSPACE_ID]: workspaceId,
	} = request.headers;
	const { verification } = request;

	//TODO getWorkspaceId(request)
	const workspaceId = getWorkspaceId(request);

	const payload = { ...request.body, ...request.query, ...request.params, ...request.body?.payload };
	const eventArgs = {
		workspaceId,
		messageId,
		payload,
		context: {
			request: { ip: request.ip },
			auth: { authType: verification.authType, authData: verification.authData?.token },
		},
	};
	await bitloopsMessage(eventArgs, mq);
	reply
		.code(202)
		.headers({ [CORS.HEADERS.ACCESS_CONTROL_ALLOW_ORIGIN]: CORS.ALLOW_ORIGIN })
		.send('OK');
};

const requestResponseHandler: RouteHandlerMethod = async function (request: requestEventRequest, reply: FastifyReply) {
	// console.log('decorated tracing', this.tracing);
	const { mq } = Services.getServices();
	const {
		[RequestHeaders.WORKFLOW_ID]: workflowId,
		[RequestHeaders.NODE_ID]: nodeId,
		[RequestHeaders.ENV_ID]: environmentId,
		[RequestHeaders.WORKFLOW_VERSION]: workflowVersion,
		// [RequestHeaders.WORKSPACE_ID]: workspaceId,
	} = request.headers;

	const { verification } = request;
	const workspaceId = getWorkspaceId(request);
	console.log('WorkspaceId', workspaceId);

	const payload = { ...request.body, ...request.query, ...request.params, ...request.body?.payload };
	const requestArgs = {
		workspaceId,
		workflowId,
		nodeId,
		workflowVersion,
		environmentId,
		payload,
		context: {
			request: { ip: request.ip },
			auth: { authType: verification.authType, authData: verification.authData?.token },
		},
	};

	const result = await bitloopsRequestResponse(requestArgs, mq);

	if (result?.headers && result?.content) {
		const statusCode = result.statusCode || 200;
		result.headers[CORS.HEADERS.ACCESS_CONTROL_ALLOW_ORIGIN] = CORS.ALLOW_ORIGIN;
		reply.code(statusCode).headers(result.headers).send(result.content);
	} else
		reply
			.code(201)
			.headers({ [CORS.HEADERS.ACCESS_CONTROL_ALLOW_ORIGIN]: CORS.ALLOW_ORIGIN })
			.send(result);
};

export default bitloopsRoutes;
