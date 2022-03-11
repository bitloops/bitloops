import admin from 'firebase-admin';

export enum FirebaseCredentialsType {
	JSON = 'JSON',
	JSON_FILE = 'JSON_FILE',
	OAUTH2 = 'OAUTH2',
}

type verifyFirebaseTokenResponse = {
	value: admin.auth.DecodedIdToken;
	error: any;
};

class FirebaseAdmin {
	private client: admin.app.App;

	constructor(
		firebaseCredentialsType: FirebaseCredentialsType,
		credentials: {
			refreshToken: string;
			json: admin.ServiceAccount;
		},
		oauth2Project?: string,
	) {
		const { refreshToken, json } = credentials;
		try {
			if (firebaseCredentialsType === FirebaseCredentialsType.JSON) {
				this.client = admin.initializeApp(
					{
						credential: admin.credential.cert(json),
					},
					json.projectId,
				);
			} else if (firebaseCredentialsType === FirebaseCredentialsType.OAUTH2) {
				this.client = admin.initializeApp(
					{
						credential: admin.credential.refreshToken(refreshToken),
					},
					oauth2Project,
				);
			} else {
				console.error(firebaseCredentialsType, 'Unimplemented');
				throw Error('Unimplemented');
			}
		} catch (error) {
			console.error(error);
			throw error;
		}
	}

	async verifyIdToken(accessToken: string): Promise<verifyFirebaseTokenResponse> {
		console.log('verifying ID token');
		try {
			const decodedToken = await this.client.auth().verifyIdToken(accessToken);
			console.log('Decoded successfully:', decodedToken);
			return { value: decodedToken, error: null };
		} catch (error) {
			console.log('Error decoding token', error.errorInfo.code);
			//error.errorInfo.code == 'auth/argument-error' (wrong signature)
			//error.errorInfo.message
			return { value: null, error: error.errorInfo };
		}
	}
}

export default FirebaseAdmin;
