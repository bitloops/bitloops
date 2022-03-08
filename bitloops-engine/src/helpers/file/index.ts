import * as fs from 'fs';

const writeStringDataToFile = async (filePath: string, data: string): Promise<[boolean, Error]> => {
	return await new Promise<[boolean, Error]>((resolve, reject) => {
		fs.writeFile(filePath, data, 'utf-8', (error: Error) => {
			if (error) {
				console.error(error);
				return reject(error);
			}
			const result: [boolean, Error] = [true, null];
			return resolve(result);
		});
	}).catch((error) => {
		const result: [boolean, Error] = [false, error];
		return result;
	});
};

export { writeStringDataToFile };
