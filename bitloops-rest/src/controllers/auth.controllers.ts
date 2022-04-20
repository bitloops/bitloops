import { RouteHandlerMethod } from 'fastify';
import { CORS, KeycloakSettings } from '../constants';
import {
	ClearAuthenticationRequest,
	FinalCallbackRequest,
	GoogleRedirectRequest,
	OAuthProviderRequest,
	RefreshTokenRequest,
} from './definitions';
import { AxiosRequestHeaders } from 'axios';
import { buildUrlWithParams, Cookie, hopRequest, toCookiesHeader } from '../utils/auth';
import { Options } from '../services';

const HEADERS = { [CORS.HEADERS.ACCESS_CONTROL_ALLOW_ORIGIN]: CORS.ALLOW_ORIGIN };
const KEYCLOAK_URI = process.env.KEYCLOAK_URI || 'http://localhost:8080';

const KEYCLOAK_GOOGLE_REDIRECT = (providerId: string): string =>
	`${KEYCLOAK_URI}/auth/realms/${providerId}/broker/google/endpoint`;

// Note: using an arrow function will break the binding of this.
export const OAuthProviderHandler: RouteHandlerMethod = async function (request: OAuthProviderRequest, reply) {
	console.log('OAuthProvider Handler', request.params);
	const { OAuthProvider } = request.params;
	{
		try {
			const { provider_id, client_id, session_uuid, workspace_id, redirect_uri } = request.query;
			const { cookiesArray, oAuthProviderLocation: googleLocation } = await this.authService.getOAuthProviderURL(
				OAuthProvider,
				{
					provider_id,
					client_id,
					session_uuid,
					workspace_id,
					redirect_uri,
				},
			);
			/**
			 * we pass these cookies to google request
			 * and then google passes them in keycloak redirect
			 * Only needed when redirect_uri sent to google is keycloak's(test-purposes)
			 */
			for (const cookie of cookiesArray) {
				reply.cookie(cookie.name, cookie.value, {
					path: `/auth/realms/${provider_id}/`,
					domain: Options.getOption(KeycloakSettings.COOKIES_DOMAIN) ?? 'localhost',
					// httpOnly: true,
				});
			}

			console.log('Redirecting to', googleLocation);
			reply.redirect(googleLocation);
		} catch (error) {
			console.log(`${OAuthProvider}-login-error`, error?.response?.data ?? error);
			return reply.code(500).headers(HEADERS).send(error.response.data);
		}
	}
};

export const authIndex: RouteHandlerMethod = async function (request, reply) {
	return reply.sendFile('login.html');
};

/**
 *  TBD how it will be used as redirect after google success login
 * instead of keycloak
 * @param request
 * @param reply
 */
// export const googleCallback: RouteHandlerMethod = async (request: GoogleRedirectRequest, reply) => {
// 	console.log('Callback start-----------------');
// 	console.log('query', request.query);
// 	// TODO get bitloops aka provider_id dynamically
// 	const keycloakRedirectURL = buildUrlWithParams(KEYCLOAK_GOOGLE_REDIRECT('bitloops'), request.query);
// 	console.log('NEW URL toString', keycloakRedirectURL);

// 	// return reply.redirect(keycloakRedirectURL);

// 	const headers: AxiosRequestHeaders = {
// 		Cookie: toCookiesHeader(cookiesMem),
// 	};
// 	console.log('headers', headers);

// 	const response = await hopRequest(keycloakRedirectURL, headers);
// 	console.log('callback-response-1', response.status);
// 	reply.code(204).send();
// };

/**
 * We get redirected here from keycloak when auth provider flow ends.
 * It exchanges a code for an access_token
 * And notifies <client> for auth success via publish-subscribe(OnAuthStateChanged)
 * and closes browser tab
 * @param request
 * @param reply
 * @returns
 */
export const keycloakOAuthProviderCallback: RouteHandlerMethod = async function (request: FinalCallbackRequest, reply) {
	const { code, session_state } = request.query;
	const { OAuthProvider } = request.params;
	try {
		const [value, error] = await this.authService.keycloakOAuthProviderCallback(OAuthProvider, { code, session_state });
		if (error) {
			return reply.code(400).headers(HEADERS).send(error.response.data);
		}
		return reply
			.headers({ ...HEADERS, 'Content-Type': 'text/html' })
			.send(`<!DOCTYPE html><html><head><script>window.close()</script></head><body></body></html>`);
	} catch (error) {
		return reply.code(500).headers(HEADERS).send(error?.message);
	}
};

export const fetchUserInfoHandler: RouteHandlerMethod = async function (
	request: GoogleRedirectRequest,
	reply,
): Promise<void> {
	const { access_token, provider_id } = request.query;
	const { data, error } = await this.authService.fetchUserInfo(access_token, provider_id);

	if (error) {
		console.error(error);
		reply.code(400).headers(HEADERS).send(data);
	} else {
		reply.code(200).headers(HEADERS).send(data);
	}
};

export const refreshTokenHandler: RouteHandlerMethod = async function (
	request: RefreshTokenRequest,
	reply,
): Promise<void> {
	const { refreshToken, clientId, providerId, sessionUuid } = request.body;
	const { data, error } = await this.authService.refreshToken({ refreshToken, clientId, providerId, sessionUuid });
	if (error) return reply.code(400).headers(HEADERS).send(data);

	reply.code(201).header('Content-Type', 'application/json; charset=utf-8').headers(HEADERS).send(data);
};

export const clearAuthenticationHandler: RouteHandlerMethod = async function (
	request: ClearAuthenticationRequest,
	reply,
): Promise<void> {
	const { accessToken, clientId, providerId, refreshToken, sessionUuid, workspaceId } = request.body;
	const { data, error } = await this.authService.clearAuthentication({
		accessToken,
		clientId,
		providerId,
		refreshToken,
		sessionUuid,
		workspaceId,
	});
	if (error) {
		reply.code(400).headers(HEADERS).send(data);
	} else {
		reply.code(204).headers(HEADERS).send();
	}
};
