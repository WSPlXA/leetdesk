"""Compile and run the exact Go snippets displayed by the offline lessons.

Only standard-library imports are used. Each solution gets its own package so
LeetCode function names and Constructor methods cannot collide across problems.
Run from the repository root: python scripts/check_lesson_go.py
"""
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SHARED = r'''
func equal(a,b any)bool{return reflect.DeepEqual(a,b)}
func bagEqual(a,b any)bool{
    canonical:=func(v any)[]string{
        r:=reflect.ValueOf(v); out:=[]string{}
        for i:=0;i<r.Len();i++{data,_:=json.Marshal(r.Index(i).Interface());out=append(out,string(data))}
        sort.Strings(out);return out
    }
    return equal(canonical(a),canonical(b))
}
func medianWindowOracle(f func([]int,int)[]float64)bool{
    for n:=1;n<=36;n++{
        for mode:=0;mode<4;mode++{
            nums:=make([]int,n)
            for i:=range nums{switch mode{case 0:nums[i]=i;case 1:nums[i]=-i;case 2:nums[i]=(i*i*7+i*3)%9-4;case 3:nums[i]=2147483647;if i%2==0{nums[i]=-2147483648}}}
            for k:=1;k<=n;k++{
                got:=f(append([]int{},nums...),k)
                if len(got)!=n-k+1{return false}
                for i:=range got{a:=append([]int{},nums[i:i+k]...);sort.Ints(a);want:=float64(a[k/2]);if k%2==0{want=(float64(a[k/2-1])+float64(a[k/2]))/2};if got[i]!=want{return false}}
            }
        }
    };return true
}
func sudokuBoard()[][]byte{
    rows:=[]string{"534678912","672195348","198342567","859761423","426853791","713924856","961537284","287419635","345286179"}
    a:=[][]byte{};for _,s:=range rows{a=append(a,[]byte(s))};return a
}
func validSudoku(a [][]byte)bool{
    var rows,cols,boxes [9]int
    for r:=0;r<9;r++{for c:=0;c<9;c++{v:=a[r][c];if v<'1'||v>'9'{return false};bit:=1<<int(v-'1');b:=r/3*3+c/3;if rows[r]&bit!=0||cols[c]&bit!=0||boxes[b]&bit!=0{return false};rows[r]|=bit;cols[c]|=bit;boxes[b]|=bit}}
    return true
}
'''
NODES = {
    '': '',
    'list': r'''
type ListNode struct{Val int;Next *ListNode}
func list(v ...int)*ListNode{d:=&ListNode{};t:=d;for _,x:=range v{t.Next=&ListNode{Val:x};t=t.Next};return d.Next}
func listValues(n *ListNode)[]int{v:=[]int{};for n!=nil{v=append(v,n.Val);n=n.Next};return v}
''',
    'tree': r'''
type TreeNode struct{Val int;Left,Right *TreeNode}
func tree(v ...any)*TreeNode{
    var build func(int)*TreeNode
    build=func(i int)*TreeNode{if i>=len(v)||v[i]==nil{return nil};return &TreeNode{Val:v[i].(int),Left:build(2*i+1),Right:build(2*i+2)}}
    return build(0)
}
func inorderValues(n *TreeNode)[]int{a:=[]int{};var dfs func(*TreeNode);dfs=func(n *TreeNode){if n==nil{return};dfs(n.Left);a=append(a,n.Val);dfs(n.Right)};dfs(n);return a}
''',
    'random': 'type Node struct{Val int;Next,Random *Node}\n',
    'graph': 'type Node struct{Val int;Neighbors []*Node}\n',
}


def main():
    data = json.loads((ROOT / 'assets/lesson-details.json').read_text(encoding='utf-8'))
    target = ROOT / 'target'
    target.mkdir(exist_ok=True)
    directory = Path(tempfile.mkdtemp(prefix='lesson-go-', dir=target))
    (directory / 'go.mod').write_text('module lessonchecks\n\ngo 1.21\n', encoding='utf-8')
    checks = 0
    for number, (key, detail) in enumerate(data.items()):
        folder = directory / f'p{number:03}'
        folder.mkdir()
        source = 'package solution\n\n' + detail['go']
        (folder / 'solution.go').write_text(source, encoding='utf-8')
        tests = '\n'.join(
            f'if !({check}) {{t.Fatal("case {i + 1} failed")}}'
            for i, check in enumerate(detail['checks'])
        )
        checks += len(detail['checks'])
        body = 'package solution\nimport("testing";"reflect";"encoding/json";"sort")\n'
        body += SHARED + NODES[detail['nodes']] + '\nfunc TestLesson(t *testing.T){\n' + tests + '\n}\n'
        (folder / 'solution_test.go').write_text(body, encoding='utf-8')
        (folder / 'lesson.txt').write_text(key, encoding='utf-8')
    env = os.environ.copy()
    env.update(GOWORK='off', GOPROXY='off', GOTOOLCHAIN='local')
    log = directory / 'results.log'
    print(f'Go lesson packages: {len(data)}, authored checks: {checks}; log: {log}', flush=True)
    process = subprocess.Popen(['go', 'test', '-timeout', '45s', '-p', '2', './...'], cwd=directory, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    passed = 0
    with log.open('w', encoding='utf-8') as output:
        for line in process.stdout:
            output.write(line)
            output.flush()
            if line.startswith('ok '):
                passed += 1
                if passed % 20 == 0:
                    print(f'Go packages passed: {passed}/{len(data)}', flush=True)
            else:
                print(line.rstrip(), flush=True)
    if process.wait():
        raise SystemExit(process.returncode)
    print('All displayed Go solutions compiled and passed their checks, including the sliding-median oracle.')


if __name__ == '__main__':
    main()
