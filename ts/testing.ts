class Test {
    internal_num: number;
    other_num = 1;

    constructor(param: number) {
        this.internal_num = param;
    }

    test: () => void = () => {};
}

function test() {
    return new Test(1);
}

const test2 = () => new Test(1);

// Ignore using

interface ITest {
    test: () => void;
}

type TTest = {
    test: () => void;
}

enum ETest {
    test = 'test'
}

module MTest {
    export function test() {}
}