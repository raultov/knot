// Alpha-side SharedUtil + caller
export class SharedUtil {
    work(): number {
        return 42;
    }
}

export function alphaCaller(): number {
    return new SharedUtil().work();
}
