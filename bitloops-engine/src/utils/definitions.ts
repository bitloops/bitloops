export type JWTData = {
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

export enum JWTStatuses {
	OK = 'ok',
	INVALID = 'invalid',
	EXPIRED = 'expired',
	ERROR = 'error',
};

export type JWT = {
	status: JWTStatuses;
	jwtData: JWTData | null;
};