import {
	buildUrlWithParams,
	Cookie,
	hopRequest,
	parseJWT,
	parseSetCookieHeader,
	replaceRedirectUrl,
	toCookiesHeader,
} from '../../utils/auth';
import axios, { AxiosRequestConfig, AxiosResponse } from 'axios';
import * as qs from 'qs';
import { IDatabase, IIMDB, IMQ } from '../interfaces';
import IAuthenticationService from './interface';
import { GRANT_TYPES } from '../../controllers/definitions';
import { getPublicKey } from '../../routes/helpers';
import { JWTStatuses } from '../../routes/definitions';
import { Options } from '..';
import { KeycloakSettings } from '../../constants';
import { OAuthProvider } from './definitions';

const replaceCookie = (cookies: Cookie[], newCookie: Cookie): Cookie[] => {
	const newCookies = cookies.filter((cookie) => cookie.name !== newCookie.name);
	newCookies.push(newCookie);
	return newCookies;
};

// TODO implement secret caching
const getClientSecret = async (providerId: string, clientId: string, db: IDatabase): Promise<string> => {
	// return Options.getOption(KeycloakSettings.CLIENT_SECRET);
	return db.getProviderClientSecret(providerId, clientId);
};

export default class AuthService implements IAuthenticationService {
	private mapper: Record<OAuthProvider, string> = {
		google: 'google',
		github: 'github',
	};
	constructor(
		private readonly imdb: IIMDB,
		private readonly mq: IMQ,
		private readonly db: IDatabase,
		private readonly REST_URI: string,
		private readonly KEYCLOAK_URI: string,
	) {}

	/**
	 * Returns the Prompt for OAuthProvider Login page
	 * and cookies of the current auth session
	 * for persistence in redis
	 */
	public async getOAuthProviderURL(
		oAuthProvider: OAuthProvider,
		{ provider_id, client_id, session_uuid, workspace_id, redirect_uri },
	) {
		const redirectUri =
			redirect_uri ?? `${this.REST_URI}/${provider_id}/auth/${this.mapper[oAuthProvider]}/final-callback`;
		console.log('redirectUri', redirectUri);
		const url = this.getAuthSessionBaseURL(provider_id);
		const authSessionURL = buildUrlWithParams(url, {
			client_id,
			response_type: 'code',
			redirect_uri: redirectUri,
		});
		const response = await axios.get(authSessionURL);
		console.log('FIRST RESPONSE', response.status);

		const cookies = response.headers['set-cookie'];
		let cookiesArray = parseSetCookieHeader(cookies);
		const authSessionCookie = cookiesArray.find((cookie) => cookie.name === 'AUTH_SESSION_ID');
		const sessionState = authSessionCookie.value.split('.')[0];
		await this.imdb.setSessionInfo(sessionState, {
			providerId: provider_id,
			clientId: client_id,
			sessionUuid: session_uuid,
			workspaceId: workspace_id,
		});
		const headers = {
			Cookie: toCookiesHeader(cookiesArray),
		};

		const keycloakOAuthProviderURL = buildUrlWithParams(this.getAuthSessionBaseURL(provider_id), {
			client_id,
			response_type: 'code',
			redirect_uri: redirectUri,
			code: 'openid',
			kc_idp_hint: this.mapper[oAuthProvider],
		});
		console.log('second link', keycloakOAuthProviderURL);
		const secondResponse = await hopRequest(keycloakOAuthProviderURL, headers);

		console.log('secondResponse', secondResponse.status);
		const newCookies = parseSetCookieHeader(secondResponse.headers['set-cookie']);
		cookiesArray = replaceCookie(cookiesArray, newCookies[0]);
		const location = secondResponse?.headers?.location;

		console.log('third link', location);
		const thirdResponse = await hopRequest(location, headers);

		console.log('thirdResponse', thirdResponse.status);
		let oAuthProviderLocation = thirdResponse?.headers?.location;

		// const REDIRECT_URI_VALUE = 'http://localhost:8080/auth/realms/bitloops/broker/google/endpoint';
		// This env is defined inside localhost containers where keycloak returns
		// an unreachable url for google(with containerName as domain)
		const redirectUrl = process.env.KEYCLOAK_FROM_GOOGLE_URI
			? `${process.env.KEYCLOAK_FROM_GOOGLE_URI}/auth/realms/${provider_id}/broker/${oAuthProvider}/endpoint`
			: `${process.env.KEYCLOAK_URI}/auth/realms/${provider_id}/broker/${oAuthProvider}/endpoint`;

		oAuthProviderLocation = replaceRedirectUrl(oAuthProviderLocation, redirectUrl);

		return {
			cookiesArray,
			oAuthProviderLocation,
		};
	}

	public async keycloakOAuthProviderCallback(
		oAuthProvider: OAuthProvider,
		{ code, session_state },
	): Promise<[boolean, any]> {
		const sessionInfo = await this.imdb.getSessionInfo(session_state);
		// console.log('got SESSION INFO', sessionInfo);

		const clientSecret = await getClientSecret(sessionInfo.providerId, sessionInfo.clientId, this.db);
		const redirectUri = `${this.REST_URI}/bitloops/auth/${this.mapper[oAuthProvider]}/final-callback`;
		const requestBody = {
			grant_type: GRANT_TYPES.AUTHORIZATION_CODE,
			code,
			client_id: sessionInfo.clientId,
			client_secret: clientSecret,
			redirect_uri: redirectUri,
		};
		const url = this.getTokenBaseURL(sessionInfo.providerId);
		const headers = {
			'Content-Type': 'application/x-www-form-urlencoded',
		};
		const params = new URLSearchParams(requestBody).toString();
		let response: AxiosResponse;
		try {
			// console.log('REQ BODY', requestBody);
			// Exchange code for access_token (based on Authorization Code flow )
			response = await axios({ url, method: 'POST', data: params, headers });
		} catch (error) {
			if (axios.isAxiosError(error)) {
				return [null, error];
			}
			throw new Error('unexpected axios error');
		}

		const PUBLIC_KEY = await getPublicKey(sessionInfo.providerId, sessionInfo.clientId);
		const jwt = parseJWT(response.data.access_token, PUBLIC_KEY);
		if (!PUBLIC_KEY || jwt.status === JWTStatuses.INVALID || JWTStatuses.ERROR) {
			if (!PUBLIC_KEY) {
				throw new Error('Error fetching required certificate information from server.');
			}
		}
		const jwtData = jwt?.jwtData;
		console.log('jwtData', jwtData);
		const payload = {
			displayName: jwtData.name,
			email: jwtData.email,
			emailVerified: jwtData.email_verified,
			firstName: jwtData.given_name,
			lastName: jwtData.family_name,
			photoURL: jwtData.photoURL,
			uid: jwtData.sub,
			accessToken: response.data.access_token,
			refreshToken: response.data.refresh_token,
			providerId: sessionInfo.providerId,
			clientId: sessionInfo.clientId,
			sessionState: response.data.session_state,
			isAnonymous: false,
		};
		const authStateChangeTopic = `workflow-events.${sessionInfo.workspaceId}.auth:${sessionInfo.providerId}:${sessionInfo.sessionUuid}`;
		console.log(`publishing to ${authStateChangeTopic}`);
		await this.mq.publish(authStateChangeTopic, {
			payload,
		});
		// console.log('tokens', JSON.stringify(response.data));
		return [true, null];
	}

	public async fetchUserInfo(
		access_token: string,
		provider_id: string,
	): Promise<{
		error: boolean;
		data: any;
	}> {
		const headers = { Authorization: `Bearer ${access_token}` };
		const url = this.getUserInfoBaseURL(provider_id);
		try {
			const userDataResponse = await axios.get(url, { headers });
			// console.log('userDataResponse', userDataResponse);
			const userData = {
				uid: userDataResponse.data.sub,
				email: userDataResponse.data.email,
				firstName: userDataResponse.data.given_name,
				lastName: userDataResponse.data.family_name,
				displayName: userDataResponse.data.name,
				emailVerified: userDataResponse.data.email_verified,
				photoURL: userDataResponse.data.photoURL,
			};
			return { error: false, data: userData };
		} catch (error) {
			if (axios.isAxiosError(error)) {
				return { error: true, data: error.response.data };
			}
			throw new Error('unexpected axios error');
		}
	}

	public async refreshToken({ refreshToken, clientId, providerId, sessionUuid }): Promise<{
		error: boolean;
		data: any;
	}> {
		const url = this.getTokenBaseURL(providerId);
		const clientSecret = await getClientSecret(providerId, clientId, this.db);

		const data = qs.stringify({
			refresh_token: refreshToken,
			client_id: clientId,
			client_secret: clientSecret,
			grant_type: GRANT_TYPES.REFRESH_TOKEN,
		});
		console.log('Refreshing token, values', data);
		try {
			const response = await axios({
				method: 'POST',
				url,
				headers: {
					'Content-Type': 'application/x-www-form-urlencoded',
				},
				data: data,
			});
			// console.log('response', response);
			const tokenData = {
				accessToken: response.data.access_token,
				expiresIn: response.data.expires_in,
				refreshExpiresIn: response.data.refresh_expires_in,
				refreshToken: response.data.refresh_token,
				tokenType: response.data.token_type,
				sessionState: response.data.session_state,
			};
			return { data: tokenData, error: null };
		} catch (error) {
			console.log('Refresh token error', error?.response?.status, error?.response?.data);
			if (sessionUuid) {
			}
			return { data: error.response.data, error };
		}
	}

	public async clearAuthentication({
		accessToken,
		clientId,
		providerId,
		refreshToken,
		sessionUuid,
		workspaceId,
	}): Promise<{
		error: boolean;
		data: any;
	}> {
		console.log('Clearing authentication');
		const url = this.getLogoutBaseURL(providerId);
		const clientSecret = await getClientSecret(providerId, clientId, this.db);
		const data = qs.stringify({
			refresh_token: refreshToken,
			client_id: clientId,
			client_secret: clientSecret,
		});
		// console.log('data', data);
		const config: AxiosRequestConfig = {
			method: 'post',
			url,
			headers: {
				Authorization: `Bearer ${accessToken}`,
				'Content-Type': 'application/x-www-form-urlencoded',
			},
			data: data,
		};
		try {
			const response = await axios(config);
			if (sessionUuid) {
				const authSessionTopic = `workflow-events.${workspaceId}.auth:${providerId}:${sessionUuid}`;
				// payload content is what gets send to the client.
				await this.mq.publish(authSessionTopic, {
					payload: {}, //clearedAuth: true,
				});
			}
			// console.log('logout response', response.data);
			return { data: response.data, error: null };
		} catch (error) {
			console.log('Clear auth Error, status:', error?.response.status);
			return { data: error.response.data, error };
		}
	}

	private getAuthSessionBaseURL(providerId: string): string {
		return `${this.KEYCLOAK_URI}/auth/realms/${providerId}/protocol/openid-connect/auth`;
	}

	private getTokenBaseURL(providerId: string): string {
		return `${this.KEYCLOAK_URI}/auth/realms/${providerId}/protocol/openid-connect/token`;
	}

	private getUserInfoBaseURL(providerId: string): string {
		return `${this.KEYCLOAK_URI}/auth/realms/${providerId}/protocol/openid-connect/userinfo`;
	}

	private getLogoutBaseURL(providerId: string): string {
		return `${this.KEYCLOAK_URI}/auth/realms/${providerId}/protocol/openid-connect/logout`;
	}
}
