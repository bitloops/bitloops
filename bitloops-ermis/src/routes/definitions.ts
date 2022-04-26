import { FastifyRequest, RawReplyDefaultExpression } from 'fastify';

import { AuthTypes, RequestHeaders } from '../constants';
import { JWTData } from '../controllers/definitions';

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

export type CacheParams = {
	cacheType: string;
}

export type CacheQuery = {
	id: string;
}

export type CacheRequest = AuthorizedRequest &
	FastifyRequest<{
		Params: CacheParams;
		Querystring?: CacheQuery;
	}>;

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