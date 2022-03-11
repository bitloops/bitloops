import { PublishHeaders } from './../constants';
import { FastifyReply, FastifyRequest } from 'fastify';
import { AppOptions, AuthTypes, CORS, KeycloakSettings } from '../constants';
import { verifyFirebase, verifyXApiKeyAuthentication } from '../helpers/auth';
import { Options } from '../services';
import { extractAuthTypeAndToken } from '../utils';
import { parseJWT } from '../utils/auth';
import { IFirebaseCredentials, JWTStatuses } from './definitions';

export const authMiddleware = async (request: FastifyRequest, reply: FastifyReply): Promise<void> => {
	// 1. User is not logged-in and resource doesn't require authorized user    => All good nothing extra needs to happen
	// 2. User is not logged-in and resource requires authorized user           => 401 is returned from the Bitloops Engine // TODO implement temp blacklisting at the REST level after several 401s
	// 3. User is logged-in and has valid access key                            => All good nothing extra needs to happen
	// 4. User is logged-in and has invalid access key but valid refresh key    => Refresh key is used to issue new access token and new refresh key
	// 5. User is logged-in and has invalid access key and invalid refresh key  => User's onAuthChange listener is triggered with logout

	// Below line is removed because of case 1. There can be requests that don't require auth
	// so we will evaluate auth only if it is present. If a resource is protected and auth info is
	// missing this is going to be evaluated and rejected at the Engine. We assume that requests are
	// usually done by Alice and Bob and not by Chad, Chuk or Mallory. If Chad, Chuk or Mallory decide
	// to make requests they will be rejected at the Engine level and REST can cache the number of
	// rejections and blacklist them eventually.
	// https://en.wikipedia.org/wiki/Alice_and_Bob

	// if (!request.headers?.authorization) return replyUnauthorized(reply);
	console.log('inserted auth middleware');
	try {
		const { authType, token } = extractAuthTypeAndToken(
			request.headers?.authorization ? request.headers?.authorization : `${AuthTypes.Unauthorized} `,
		);
		const providerId = request.headers['provider-id']?.toString();

		switch (authType.toLocaleLowerCase()) {
			/**
			 * Basic username password authentication
			 * Very similar to X API Key implementation but using a different collection
			 */
			case AuthTypes.Basic.toLocaleLowerCase():
				return handleUnimplemented();
			case AuthTypes.X_API_KEY.toLocaleLowerCase():
				return await handleXApiKey(request, reply, token);
			/**
			 * OAuth 2
			 * To be implemented last
			 */
			case AuthTypes.Token.toLocaleLowerCase():
				return handleUnimplemented();
			case AuthTypes.FirebaseUser.toLocaleLowerCase(): {
				const firebaseCredentials: IFirebaseCredentials = { token, providerId };
				return await handleFirebaseUser(request, reply, firebaseCredentials);
			}
			case AuthTypes.User.toLocaleLowerCase():
				return handleUser(request, reply);
			case AuthTypes.Anonymous.toLocaleLowerCase():
				return handleAnonymousUser(request);
			case AuthTypes.Unauthorized.toLocaleLowerCase():
				return handleUnauthorized(request);
			default:
				console.log('Unauthenticated connection');
			// return replyUnauthorized(reply);
		}
	} catch (error) {
		// maybe reply about authorization header correct format
		console.error(error);
		replyUnauthorized(reply);
	}
};

function handleUnimplemented() {
	throw new Error('Unimplemented auth type');
}

function handleAnonymousUser(request: any) {
	request.verification = {
		authType: AuthTypes.Anonymous,
	};
}

export const getPublicKey = async (providerId: string, clientId: string): Promise<string> => {
	// TODO get public key using providerId and clientId from Mongo or local cache
	const base64PublicKey = Options.getOption(KeycloakSettings.PUBLIC_KEY);
	const publicKeyString = Buffer.from(base64PublicKey, 'base64').toString();
	return publicKeyString;
};

const handleUser = async (request: any, reply: any) => {
	const { token } = extractAuthTypeAndToken(request.headers?.authorization);
	const providerId = request.headers['provider-id'];
	const clientId = request.headers['client-id'];
	const PUBLIC_KEY = await getPublicKey(providerId, clientId);
	const jwt = parseJWT(token, PUBLIC_KEY);
	console.log('jwt status', jwt.status);
	request.verification = AuthTypes.User;
	if (jwt.status === JWTStatuses.OK) {
		request.verification = {
			authType: AuthTypes.User,
			authData: {
				token,
				decodedToken: jwt,
			},
		};
	} else replyUnauthorized(reply);
};

const handleFirebaseUser = async (request, reply: FastifyReply, firebaseCredentials: IFirebaseCredentials) => {
	const { token, providerId } = firebaseCredentials;
	const firebaseVerification = await verifyFirebase(token, providerId);
	const emailArray = firebaseVerification?.email.split('@');
	if (firebaseVerification.status === Number(AppOptions.UNAUTHORIZED_STATUS)) {
		return replyUnauthorized(reply);
	}
	request.verification = {
		authType: AuthTypes.FirebaseUser,
		authData: {
			token,
			decodedToken: firebaseVerification,
		},
	};
};

export const handleXApiKey = async (request, reply: FastifyReply, token: string) => {
	console.log('entered x api-key', token);
	const verification = await verifyXApiKeyAuthentication(token);
	if (verification.status === Number(AppOptions.UNAUTHORIZED_STATUS)) {
		return replyUnauthorized(reply);
	}
	request.verification = {
		authType: AuthTypes.X_API_KEY,
		authData: {
			token,
			decodedToken: verification,
		},
	};
};

export const getWorkspaceId = (request: any) => {
	let workspaceId = request?.verification?.workspaceId;
	/** From firebase */
	if (!workspaceId)
		workspaceId = request.body?.workspaceId ?? request.query?.workspaceId ?? request.params?.workspaceId;
	return workspaceId;
};

export const replyUnauthorized = (reply: FastifyReply) => {
	reply
		.code(401)
		.type('text/xml')
		.header('WWW-Authenticate', 'Basic realm="Bitloops API"')
		.header(CORS.HEADERS.ACCESS_CONTROL_ALLOW_ORIGIN, CORS.ALLOW_ORIGIN)
		.send('<?xml version="1.0" encoding="UTF-8"?><Error>401 Unauthorized</Error>');
};

const handleUnauthorized = (request: any) => {
	request.verification = {
		authType: AuthTypes.Unauthorized,
	};
};
