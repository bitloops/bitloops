import { AppOptions, AuthTypes } from './../../constants';
import { bitloopsRequestResponse } from '../../bitloops';
import { KeycloakSettings, RequestHeaders } from '../../constants';
import { getWorkspaceId } from '../../routes/helpers';
import Services, { Options } from '../../services';
import { expired } from '../../utils';

export const getPublicKey = async (providerId: string): Promise<string> => {
	const { publicKeysCache } = Services.getServices();
	const cachedPK = publicKeysCache.fetch(providerId);
	// console.log('cache has: ', cachedXApiKey);
	let publicKey: string | null = cachedPK?.pk ?? null;
	if (cachedPK === null || expired(cachedPK)) {
		console.log('Public key not cached or expired:', providerId);
		publicKey = await getPublicKeyWorkflow(providerId);
		if (publicKey === null) {
			return null;
		}
		publicKeysCache.cache(providerId, publicKey);
	}
	// return Buffer.from(publicKey, 'base64').toString();
	return publicKey;
	// const base64PublicKey = Options.getOption(KeycloakSettings.PUBLIC_KEY);
	// const publicKeyString = Buffer.from(base64PublicKey, 'base64').toString();
	// return publicKeyString;
};

const getPublicKeyWorkflow = async (providerId: string): Promise<string | null> => {
	const { mq } = Services.getServices();

	const requestArgs = {
		workspaceId: '', // TODO Fill our workspaceId here?
		workflowId: '3408dc57-fd96-4e0e-b368-667a4f0715a3',
		nodeId: 'fbb154c9-b36a-4012-a528-857d59a23e1f',
		environmentId: Options.getOption(AppOptions.ENVIRONMENT_ID) ?? 'production',
		payload: {
			providerId,
		},
		context: {
			auth: { authType: AuthTypes.Unauthorized },
		},
	};
	// console.log('requestArgs', requestArgs);
	const response = await bitloopsRequestResponse(requestArgs, mq);
	// console.log('get public key res', response);
	return response?.content?.publicKey ?? null;
};
