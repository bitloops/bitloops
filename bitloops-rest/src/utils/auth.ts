import * as crypto from 'crypto';
import base64url from 'base64url'; // TODO consider if we should replace it with internal code to reduce deps
import { JWT, JWTStatuses } from '../routes/definitions';
import { JWTData } from '../controllers/definitions';

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

	// const jwtData = JSON.parse(Buffer.from(jwtPayload, 'base64').toString());
	const base64Payload = jwtPayload.replace(/-/g, '+').replace(/_/g, '/');
	const jsonPayload = decodeURIComponent(
		Buffer.from(base64Payload, 'base64')
			.toString()
			.split('')
			.map((c) => {
				return '%' + ('00' + c.charCodeAt(0).toString(16)).slice(-2);
			})
			.join(''),
	);

	const jwtData = JSON.parse(jsonPayload) as JWTData;
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

import axios, { AxiosRequestHeaders } from 'axios';

type Cookie = {
	name: string;
	value: string;
};

const toCookiesHeader = (cookies: Cookie[]): string => {
	let cookieString = '';
	for (const [index, cookie] of cookies.entries()) {
		cookieString += `${cookie.name}=${cookie.value}`;
		if (index !== cookies.length - 1) {
			cookieString += '; ';
		}
	}
	return cookieString;
};

const parseSetCookieHeader = (cookies: string[]): Cookie[] => {
	//   const headers = {
	//     Cookie:
	//       'AUTH_SESSION_ID_LEGACY=e13447a8-2e2c-4b2f-a798-b572eed34d52.75cbb82501c0; KC_RESTART=eyJhbGciOiJIUzI1NiIsInR5cCIgOiAiSldUIiwia2lkIiA6ICIxMDU2YmUxMy1iNTkwLTQ2MjctOTAzZS0wMzRkYjNkOTc1NTgifQ.eyJjaWQiOiJ0ZXN0X2NsaWVudCIsInB0eSI6Im9wZW5pZC1jb25uZWN0IiwicnVyaSI6Imh0dHA6Ly8xMjcuMC4wLjE6NDIwMC9jYWxsYmFjayIsImFjdCI6IkFVVEhFTlRJQ0FURSIsIm5vdGVzIjp7ImlzcyI6Imh0dHA6Ly9sb2NhbGhvc3Q6ODA4MC9hdXRoL3JlYWxtcy9iaXRsb29wcyIsInJlc3BvbnNlX3R5cGUiOiJjb2RlIiwicmVkaXJlY3RfdXJpIjoiaHR0cDovLzEyNy4wLjAuMTo0MjAwL2NhbGxiYWNrIn19.gQ1HmFeWRO4XulGwqkQzyY9l819_0bGYfJHvjkr4vQk',
	//   };
	const cookiesArray: Cookie[] = [];
	for (const [index, cookie] of cookies.entries()) {
		const parts = cookie.split(';');
		const nameValue = parts[0];
		const [name, value] = nameValue.split('=');
		cookiesArray.push({ name, value });
	}
	return cookiesArray;
	// const headers = {
	// 	Cookie: cookieString,
	// };
	// console.log('Headers', headers);

	// return headers;
};

const hopRequest = async (url: string, headers: AxiosRequestHeaders) => {
	try {
		const response = await axios.get(url, { headers, maxRedirects: 0 });
		return response;
	} catch (error) {
		if (axios.isAxiosError(error)) {
			return error.response;
		}
		console.error(error);
		throw new Error('unexpected axios error');
	}
};

const replaceRedirectUrl = (originalUrl: string, value: string): string => {
	//redirect_uri
	const REDIRECT_URI_PARAM = 'redirect_uri';

	// const REDIRECT_URI_VALUE = 'http://localhost:3005/bitloops/auth/google/callback';
	const url = new URL(originalUrl);
	const originalRedirect = url.searchParams.get(REDIRECT_URI_PARAM);
	url.searchParams.set(REDIRECT_URI_PARAM, value);
	console.log('OLD url\n', originalUrl);
	console.log('REPLACED url\n', url.href);
	return url.href;
};

const buildUrlWithParams = (baseUrl: string, params: Record<string, string>): string => {
	const url = new URL(baseUrl);
	url.search = new URLSearchParams(params).toString();
	return url.toString();
};

export { Cookie, parseSetCookieHeader, hopRequest, replaceRedirectUrl, toCookiesHeader, buildUrlWithParams };
