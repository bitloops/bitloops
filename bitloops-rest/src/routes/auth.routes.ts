import { FastifyInstance } from 'fastify';
import * as AuthController from '../controllers/auth.controllers';
import { IDatabase, IIMDB, IMQ } from '../services/interfaces';
import AuthService from '../services/Auth/auth.service';
import { OAuthProviderParams } from './definitions';

const authRoutes = (mq: IMQ, imdb: IIMDB, db: IDatabase) => async (fastify: FastifyInstance, _opts, done) => {
	const REST_URI = process.env.REST_URI || 'http://localhost:3005';
	const KEYCLOAK_URI = process.env.KEYCLOAK_URI || 'http://localhost:8080';
	const authService = new AuthService(imdb, mq, db, REST_URI, KEYCLOAK_URI);
	fastify.decorate('authService', authService);

	fastify
		.get('/providers/:providerId/protocol/openid-connect/auth', AuthController.authIndex)
		.post('/userInfo', AuthController.fetchUserInfoHandler)
		.post('/refreshToken', AuthController.refreshTokenHandler)
		.post('/clearAuthentication', AuthController.clearAuthenticationHandler)
		/**
		 * .e.g /google or /github
		 */
		.get(
			'/:OAuthProvider',
			{
				schema: {
					params: OAuthProviderParams,
				},
			},
			AuthController.OAuthProviderHandler,
		)
		// .get('/google/callback', AuthController.googleCallback)
		.get(
			'/:OAuthProvider/final-callback',
			{
				schema: {
					params: OAuthProviderParams,
				},
			},
			AuthController.keycloakOAuthProviderCallback,
		);
};

export default authRoutes;
