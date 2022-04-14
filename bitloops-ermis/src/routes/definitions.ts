import { FastifyRequest, RawReplyDefaultExpression } from 'fastify';

// import { ServerResponse } from 'http';
import { AuthTypes, RequestHeaders } from '../constants';
import { JWTData } from '../controllers/definitions';

// export interface TwilioEventRequest {
// 	event: string;
// }

export type RawReply = RawReplyDefaultExpression;

export interface IEventParams {
	connectionId: string;
}

export type AuthVerificationRequest = {
	authType: AuthTypes;
	authData?: {
		token: string;
		decodedToken: any;
	};
};

export type AuthorizedRequest = {
	verification: AuthVerificationRequest;
};

export type EventRequest = AuthorizedRequest &
	FastifyRequest<{
		Params: IEventParams;
	}>;

export type requestEventRequest = AuthorizedRequest &
	FastifyRequest<{
		Headers: {
			[RequestHeaders.WORKFLOW_ID]: string;
			[RequestHeaders.ENV_ID]?: string;
			[RequestHeaders.WORKFLOW_VERSION]?: string;
			[RequestHeaders.WORKSPACE_ID]?: string;
		};
		Body: any;
		Querystring: any;
		Params: any;
	}>;

// export type publishEventRequest = AuthorizedRequest &
// 	FastifyRequest<{
// 		Headers: {
// 			[PublishHeaders.MESSAGE_ID]: string;
// 			[RequestHeaders.WORKSPACE_ID]?: string;
// 		};
// 		Body: any;
// 		Querystring: any;
// 		Params: any;
// 	}>;

export const JWTStatuses = {
	OK: 'ok',
	INVALID: 'invalid',
	EXPIRED: 'expired',
	ERROR: 'error',
};

export type JWT = {
	status: string;
	jwtData: JWTData | null;
};

// export interface IFirebaseCredentials {
// 	token: string;
// 	providerId: string;
// }

// // export interface ISubscribeRequest {
// // 	Params: { connectionId: string };
// // 	Body: { topics: string[]; workspaceId: string };
// // }
export type SubscribeRequest = AuthorizedRequest &
	FastifyRequest<{
		Params: { connectionId: string };
		Body: { topics: string[]; workspaceId: string };
	}>;

export type UnSubscribeRequest = AuthorizedRequest &
	FastifyRequest<{
		Params: { connectionId: string };
		Body: { topic: string; workspaceId: string };
	}>;

// export type PENDING = 'pending';
// type connectionId = string;
// export type TTopicToConnections = Record<string, connectionId[]>;
// export type TSseConnectionIds = Record<connectionId, ServerResponse>;

// export type SrResponse = { result: any; error: Error };

// export const PostSubscribeEventsBody = {
// 	type: 'object',
// 	additionalProperties: false, // it will remove all the field that is NOT in the JSON schema
// 	properties: {
// 		topics: { type: 'array', items: { type: 'string' }, minItems: 1 },
// 		workspaceId: { type: 'string', format: 'uuid' },
// 	},
// 	required: ['topics', 'workspaceId'],
// };

// export const PostSubscribeEventsParams = {
// 	type: 'object',
// 	additionalProperties: false,
// 	properties: {
// 		connectionId: { type: 'string', oneOf: [{ format: 'uuid' }, { const: '' }] },
// 	},
// 	required: ['connectionId'],
// };

// export const AuthHeadersSchema = {
// 	type: 'object',
// 	properties: {
// 		Authorization: { type: 'string' },
// 	},
// 	required: ['Authorization'],
// };

// export const EventsParams = {
// 	type: 'object',
// 	additionalProperties: false,
// 	properties: {
// 		connectionId: { type: 'string', format: 'uuid' },
// 	},
// 	required: ['connectionId'],
// };

// export const PublishHeadersSchema = {
// 	type: 'object',
// 	properties: {
// 		'message-id': { type: 'string' },
// 	},
// 	required: ['message-id'],
// };

// export const RequestHeadersSchema = {
// 	type: 'object',
// 	properties: {
// 		'workflow-id': { type: 'string', format: 'uuid' },
// 		'node-id': { type: 'string', format: 'uuid' },
// 		'environment-id': { type: 'string' },
// 		'workflow-version': { type: 'string' },
// 	},
// 	required: ['workflow-id', 'node-id'],
// };

// export const OAuthProviderParams = {
// 	type: 'object',
// 	additionalProperties: false,
// 	properties: {
// 		OAuthProvider: { type: 'string', enum: ['google', 'github'] },
// 	},
// 	required: ['OAuthProvider'],
// };
