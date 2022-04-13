import { AppOptions } from '../../constants';
import Services, { Options } from '../../services';
import { getHash } from '../../utils/crypto';

const expired = (cachedKey: any) => {
	// TODO make it generic(not only for x_api_key)
	return (
		Date.now() - cachedKey.cached_at > Options.getOptionAsNumber(AppOptions.X_API_KEY_CACHE_TIMEOUT, 1000 * 60 * 10)
	);
};

async function getXApiKey(hash: string, services) {
	const { xApiKeyCache, db } = services;
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

// TODO fix this
interface IXApiKeyDefinition {
	id: string;
	status: number;
}
export const verifyXApiKeyAuthentication = async (token: string): Promise<IXApiKeyDefinition> => {
	console.log('received token', token);
	// const { xApiKeyCache, db } = Services.getServices();
	// const services = { xApiKeyCache, db };
	// const hash = getHash(token);
	// console.log('hash', hash);
	// const cachedXApiKey = await getXApiKey(hash, services);
	// console.log('cachedXApiKey', cachedXApiKey);
	// return cachedXApiKey;
	return {
		id: "cachedXApiKey",
		status: 200,
	};
};