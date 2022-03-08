/** @type {import('@ts-jest/dist/types').InitialOptionsTsJest} */
module.exports = {
	preset: 'ts-jest',
	testEnvironment: 'node',
	moduleNameMapper: {
		'@exmpl/(.*)': '<rootDir>/src/$1',
	},
	setupFiles: ['<rootDir>/jest-setup.ts'],
};
