import HTTPErrorResponse from './http';

export const sleep = (ms: number) => {
	return new Promise((resolve) => setTimeout(resolve, ms));
};

export const extractAuthTypeAndToken = (str: string) => {
	const [authType, token] = str.split(' ');
	if (authType) authType.toLowerCase();
	return { authType, token };
};

export { HTTPErrorResponse };
