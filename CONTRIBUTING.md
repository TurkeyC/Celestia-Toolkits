# 开发方式

## 全新开发(基于blank分支)

```zsh
# 只克隆blank分支
git clone --single-branch -b blank https://github.com/TurkeyC/Celestia-Toolkits.git
# 进入项目目录
cd Celestia-Toolkits
# 基于blank分支创建新的项目分支
git switch -c project
# 然后在项目分支中进行正常开发
# 推送到仓库并创建分支
git push -u origin project
```

## 增量开发(基于别人的项目进行的二次开发)

```zsh
# 先在别人的项目分支中进行正常开发
# 添加远程仓库
git remote add celes https://github.com/TurkeyC/Celestia-Toolkits.git
# 直接推，把当前项目的mian作为远程项目的project
git push -u celes project
```
