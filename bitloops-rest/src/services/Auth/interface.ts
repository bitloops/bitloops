import { AxiosError } from 'axios';
import { Cookie } from '../../utils/auth';
import { OAuthProvider } from './definitions';

export default interface IAuthenticationService {
	getOAuthProviderURL(
		oAuthProvider: OAuthProvider,
		{ provider_id, client_id, session_uuid, workspace_id, redirect_uri },
	): Promise<{
		cookiesArray: Cookie[];
		oAuthProviderLocation: string;
	}>;
	keycloakOAuthProviderCallback(oAuthProvider: OAuthProvider, { code, session_state }): Promise<[boolean, any]>;

	fetchUserInfo(
		access_token: string,
		provider_id: string,
	): Promise<{
		error: boolean;
		data: any;
	}>;

	refreshToken({ refreshToken, clientId, providerId, sessionUuid }): Promise<{
		error: boolean;
		data: any;
	}>;

	clearAuthentication({ accessToken, clientId, providerId, refreshToken, sessionUuid, workspaceId }): Promise<{
		error: boolean;
		data: any;
	}>;
}
