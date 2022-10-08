#!/bin/bash
# try install 3 times

function finish() {
    echo "--- PLEASE RUN sh -x ci/01-pre-commit.sh FIRST IN YOUR LOCALHOST!!! ---"
    # remove tmp
    set +x
    for rustlist in `git diff master --stat | awk '{print $1}' | grep \.rs$ | tr '\n' ' '`
    do
    sed -i '/#!\[deny(missing_docs)]/d' $rustlist 2>/dev/null || true
    sed -i '/#!\[deny(clippy::all)]/d' $rustlist 2>/dev/null || true
    sed -i '/#!\[deny(warnings)]/d' $rustlist 2>/dev/null || true
    done
}

trap finish EXIT

rustlist=`git diff master --stat | awk '{print $1}' | grep \.rs$ | tr '\n' ' '`
grep -P '[\p{Han}]' $rustlist && echo "rust 源码文件中禁用中文字符" && exit 1

pip3 install pre-commit -i http://mirrors.aliyun.com/pypi/simple/ || pip3 install  -i https://pypi.tuna.tsinghua.edu.cn/simple/ pre-commit || pip3 install pre-commit

## one PR ? Commit
# oldnum=`git rev-list origin/master --no-merges --count`
# newnum=`git rev-list HEAD --no-merges --count`
# changenum=$[newnum - oldnum]

# add doc for src code
for rustlist in `git diff master --stat | awk '{print $1}' | grep \.rs$ | tr '\n' ' '`
do
egrep '#!\[deny\(missing_docs\)\]' $rustlist || sed -i '1i\#![deny(missing_docs)]' $rustlist 2>/dev/null || true
egrep '#!\[deny\(clippy::all\)\]' $rustlist || sed -i '1i\#![deny(clippy::all)]' $rustlist 2>/dev/null || true
egrep '#!\[deny\(warnings\)\]' $rustlist || sed -i '1i\#![deny(warnings)]' $rustlist 2>/dev/null || true
done

# run base check
filelist=`git diff master --stat | awk '{print $1}' | tr '\n' ' '`
export PATH="$PATH:/home/jenkins/.local/bin"
pre-commit run -vvv --files ${filelist}
