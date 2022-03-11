import { tokenInfo } from './../../services/definitions/index';
import admin from 'firebase-admin';

import { AppOptions, BITLOOPS_PROVIDER_ID } from '../../constants';
import Services, { Options } from '../../services';
import FirebaseAdmin, { FirebaseCredentialsType } from '../../services/FirebaseAdmin';
import { getHash } from '../../utils/crypto';

const expired = (cachedKey: any) => {
	// TODO make it generic(not only for x_api_key)
	return (
		Date.now() - cachedKey.cached_at > Options.getOptionAsNumber(AppOptions.X_API_KEY_CACHE_TIMEOUT, 1000 * 60 * 10)
	);
};

const expiredJwt = (cachedKey: tokenInfo, expirationPeriodMs = 600000) => {
	// const expirationPeriodMs = Options.getOptionAsNumber(AppOptions.X_API_KEY_CACHE_TIMEOUT, 1000 * 60 * 10);
	const currentMoment = Date.now();
	const expAtMs = cachedKey.decoded_token.exp * 1000;
	const hasCachingExpired = currentMoment - cachedKey.cached_at > expirationPeriodMs;
	const hasTokenExpired = currentMoment > expAtMs;
	return hasTokenExpired || hasCachingExpired;
};

async function getXApiKey(hash: string, services) {
	const { firebaseConnectionsCache, xApiKeyCache, db } = services;
	const cachedXApiKey = await xApiKeyCache.fetch(hash);
	// console.log('cache has: ', cachedXApiKey);
	if (cachedXApiKey === null || expired(cachedXApiKey)) {
		console.log('API Key not found in cache or expired, checking db...');
		let dbXApiKey = await db.getXApiKey(hash);
		if (dbXApiKey === null) {
			dbXApiKey = {
				id: hash,
				workspaceId: '',
				cached_at: Date.now(),
				status: Number(AppOptions.UNAUTHORIZED_STATUS),
			};
			xApiKeyCache.cache(hash, dbXApiKey);
		} else {
			const objectForCache = {
				id: dbXApiKey._id,
				name: dbXApiKey.name,
				workspaceId: dbXApiKey.workspaceId,
				cached_at: Date.now(),
				created_at: dbXApiKey.created_at,
				status: Number(AppOptions.AUTHORIZED_STATUS),
			};
			xApiKeyCache.cache(hash, objectForCache);
			dbXApiKey = objectForCache;
		}
		console.log('dbXApiKey', dbXApiKey);
		return dbXApiKey;
	}
	return cachedXApiKey;
}

export const verifyXApiKeyAuthentication = async (token: string) => {
	console.log('received token', token);
	const { firebaseConnectionsCache, xApiKeyCache, db } = Services.getServices();
	const services = { firebaseConnectionsCache, xApiKeyCache, db };
	const hash = getHash(token);
	console.log('hash', hash);
	const cachedXApiKey = await getXApiKey(hash, services);
	// console.log('cachedXApiKey', cachedXApiKey);
	return cachedXApiKey;
};

export const verifyFirebase = async (
	token: string,
	providerId: string,
): Promise<admin.auth.DecodedIdToken | { status: number, email: string }> => {
	console.log('verifyFirebase ->  providerId', providerId);
	if (!providerId) return { status: Number(AppOptions.UNAUTHORIZED_STATUS), email: undefined };

	const { firebaseConnectionsCache, firebaseTokensCache, db } = Services.getServices();
	let firebaseConnection = firebaseConnectionsCache.fetch(providerId);
	console.log('firebase cache connection', firebaseConnection);
	if (!firebaseConnection) {
		if (providerId === Options.getOption(BITLOOPS_PROVIDER_ID)) {
			console.log('bitloops provider id', providerId);
			firebaseConnection = new FirebaseAdmin(FirebaseCredentialsType.JSON_FILE, {
				refreshToken: undefined,
				json: undefined,
			});
		} else {
			console.log('client provider id', providerId);
			const findResult = await db.getFirebaseCredentials(providerId);
			const credentials = findResult?.credentials;
			if (!credentials) return { status: Number(AppOptions.UNAUTHORIZED_STATUS), email: undefined };
			firebaseConnection = new FirebaseAdmin(FirebaseCredentialsType.JSON, {
				refreshToken: undefined,
				json: credentials,
			});
		}

		firebaseConnectionsCache.cache(providerId, firebaseConnection);
	}

	const cachedTokenInfo = firebaseTokensCache.fetch(token);

	if (cachedTokenInfo && !cachedTokenInfo.valid) {
		console.log('token was not valid');
		return {
			status: Number(AppOptions.UNAUTHORIZED_STATUS),
      email: undefined,
		};
	}
	if (!cachedTokenInfo || expiredJwt(cachedTokenInfo)) {
		console.log('token was not cached, or expired');
		const { error, value } = await firebaseConnection.verifyIdToken(token);
		if (error) {
			// if (error.code === 'auth/argument-error') {
			firebaseTokensCache.cache(token, { valid: false });
			// }
			return {
				status: Number(AppOptions.UNAUTHORIZED_STATUS),
        email: undefined,
			};
		}
		// can use actual exp time here
		firebaseTokensCache.cache(token, { valid: true, decoded_token: value });
		return value;
	}
	console.log('token was cached and valid here');
	return cachedTokenInfo.decoded_token;
};
