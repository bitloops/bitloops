import * as crypto from 'crypto';
import base64url from 'base64url';
import jwt_decode from 'jwt-decode';
import { JWT, JWTStatuses, JWTData } from './definitions';

/**
 * Converts the encoded JWT token and verifies its validity.
 * If the JWT is invalid, null is returned
 * If there is an issue with the public certificate
 * undefined is returned instead
 * @param token string
 * @param publicKey string
 * @returns JWT token information
 */
export const parseJWT = (token: string, publicKey: string): JWT | null => {
	const verifyFunction = crypto.createVerify('RSA-SHA256');
	const jwtHeaders = token.split('.')[0];
	const jwtPayload = token.split('.')[1];
	const jwtSignature = token.split('.')[2];
	verifyFunction.write(jwtHeaders + '.' + jwtPayload);
	verifyFunction.end();
	try {
		const jwtSignatureBase64 = base64url.toBase64(jwtSignature);
		const signatureIsValid = verifyFunction.verify(publicKey, jwtSignatureBase64, 'base64');
		if (!signatureIsValid) {
			console.error('invalid signature');
			return {
				status: JWTStatuses.INVALID,
				jwtData: null,
			};
		}
	} catch (error) {
		console.error('error with signature', error);
		return {
			status: JWTStatuses.ERROR,
			jwtData: null,
		};
	}

	const jwtData = jwt_decode<JWTData>(token);
	const { exp } = jwtData;
	const expired = Date.now() >= exp * 1000;
	if (expired) {
		return {
			status: JWTStatuses.EXPIRED,
			jwtData: null,
		};
	}
	return {
		status: JWTStatuses.OK,
		jwtData,
	};
};

export const isJWTExpired = (jwt: JWT): boolean => {
	const { status } = jwt;
	if (status === JWTStatuses.EXPIRED) return true;
	return false;
};

export const isJWTInvalid = (jwt: JWT): boolean => {
	const { status } = jwt;
	if (status === JWTStatuses.OK) return false;
	return true;
};
