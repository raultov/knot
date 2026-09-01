// Beta-side SharedUtil + caller (distinct body from alpha)
export class SharedUtil {
    work(): number {
        return 99;
    }
}

export function betaCaller(): number {
    return new SharedUtil().work();
}
