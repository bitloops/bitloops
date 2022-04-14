import { JWTData } from '../../controllers/definitions';

export type OAuthProvider = 'google' | 'github';
export type BitloopsUser = {
	accessToken: string;
	refreshToken: string;
	sessionState: string;
	uid: string;
	displayName: string;
	firstName: string;
	lastName: string;
	email: string;
	emailVerified: boolean;
	isAnonymous: boolean;
	providerId: string;
	clientId: string;
	photoURL: string;
	jwt?: JWTData;
};
