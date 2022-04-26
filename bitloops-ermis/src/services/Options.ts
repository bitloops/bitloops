class Options {
    private static options = {};
    private static serverUUID;

    private static isNumeric(str: string): boolean {
        if (typeof str != 'string') return false; // to only accept strings
        return (
            !isNaN(+str) && // use type coercion to parse the string (`parseFloat` alone does not do this)
            !isNaN(parseFloat(str))
        ); // for whitespace strings to fail
    }

    static setOption(key: string, value: string): void {
        this.options[key] = value;
    }

    static getOption(key: string): string {
        if (!this.options[key]) this.options[key] = process.env[key];
        return this.options[key];
    }

    static getOptionAsNumber(key: string, defaultValue: number): number {
        if (this.options[key]) return this.options[key];
        if (Options.isNumeric(process.env[key])) return +process.env[key];
        return defaultValue;
    }

    static setServerUUID(uuid: string): void {
        this.serverUUID = uuid;
    }

    static getServerUUID(): string {
        return this.serverUUID;
    }
}

export default Options;
