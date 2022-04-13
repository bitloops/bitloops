// import { FastifyRequest } from 'fastify';
// import { OAuthProvider } from '../services/Auth/definitions';

export enum GRANT_TYPES {
	AUTHORIZATION_CODE = 'authorization_code',
	REFRESH_TOKEN = 'refresh_token',
}

// interface IQuerystring {
// 	[key: string]: string;
// 	state: string;
// 	code: string;
// 	scope: string;
// 	authuser: string;
// 	hd: string;
// 	prompt: string;
// }

// type OAuthProviderRequest = FastifyRequest<{
// 	Querystring: {
// 		client_id: string;
// 		provider_id: string;
// 		session_uuid: string;
// 		workspace_id: string;
// 		redirect_uri?: string;
// 	};
// 	Params: {
// 		OAuthProvider: OAuthProvider;
// 	};
// }>;

// type GoogleRedirectRequest = FastifyRequest<{
// 	Querystring: IQuerystring;
// }>;

// type RefreshTokenRequest = FastifyRequest<{
// 	Body: {
// 		refreshToken: string;
// 		clientId: string;
// 		providerId: string;
// 		sessionUuid: string;
// 	};
// }>;

// type ClearAuthenticationRequest = FastifyRequest<{
// 	Body: {
// 		accessToken: string;
// 		clientId: string;
// 		providerId: string;
// 		refreshToken: string;
// 		sessionUuid: string;
// 		workspaceId: string;
// 	};
// }>;

// type FinalCallbackRequest = FastifyRequest<{
// 	Querystring: { session_state: string; code: string };
// 	Params: {
// 		OAuthProvider: OAuthProvider;
// 	};
// }>;

type JWTData = {
	exp: number;
	iat: number;
	auth_time: number;
	jti: string;
	iss: string;
	aud: string;
	sub: string;
	typ: string;
	azp: string;
	session_state: string;
	acr: string;
	realm_access: {
		roles: string[];
	};
	resource_access: { account: { roles: any } };
	scope: string;
	sid: string;
	email_verified: boolean;
	name: string;
	preferred_username: string;
	given_name: string;
	family_name: string;
	email: string;
	photoURL: string;
};

export {
	// ClearAuthenticationRequest,
	// OAuthProviderRequest,
	// GoogleRedirectRequest,
	// FinalCallbackRequest,
	// RefreshTokenRequest,
	JWTData,
};
