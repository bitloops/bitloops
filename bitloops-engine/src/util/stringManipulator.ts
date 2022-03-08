import { MatchingInfoType } from "./types";

const replaceAll = (str: string, find: string, replace: string) => {
	return str.replace(new RegExp(find, 'g'), replace);
}

/** 
 * @param str variable to be replaced
 * @param find string characters to find
 * @param replace value to replace the found values
 * 
 * @returns the result after the replaces 
 */
export const replaceAllCharacters = (str: string, find: string, replace: string) => {
	const characterFind = `[${find}]`;
	return replaceAll(str, characterFind, replace);
}

// https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/String/replace#specifying_a_function_as_a_parameter
export const replaceStringBetween = (matchingInfo: MatchingInfoType, replaceFunction: (arg1: any, arg2: any) => string): string => {
	const { str, before, after } = matchingInfo;
	const regExp = new RegExp(before + '(.*?)' + after, 'g');
	const res = str.replace(regExp, replaceFunction);
	return res;
}