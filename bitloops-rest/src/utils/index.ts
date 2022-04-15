export const sleep = (ms: number) => {
	return new Promise((resolve) => setTimeout(resolve, ms));
};

export const extractAuthTypeAndToken = (str: string) => {
	const [authType, token] = str.split(' ');
	if (authType) authType.toLowerCase();
	return { authType, token };
};

export const expired = (cachedKey: Required<{ cached_at: number }>, cachingDurationOption?: string) => {
	return Date.now() - cachedKey.cached_at > Options.getOptionAsNumber(cachingDurationOption, 1000 * 60 * 10);
};

import { AppOptions } from '../constants';
import { Options } from '../services';
import HTTPErrorResponse from './http';
export { HTTPErrorResponse };
