from sys import argv
def FileToList(projects : list):
    projectList = []
    for projectDataidx in range(0,len(projects),2):
        name = projects[projectDataidx].strip('[').strip(']')
        path = projects[projectDataidx + 1].split("=")
        projectList.append([name, path[1]])

def help(FilePath):
    print("Rattle - Project Manager Written In python")
    print("Usage: rattle [command] [projectName | filePath]")
    print("help\t- shows this message")
    print("show\t- will show your project")
    print("create\t- will create a project in folder of your choosing (req. filePath)")
    print("delete\t- will delete project of your choosing (req. projectName)")
def show(FilePath):
    with open(FilePath, "r") as f:
        lines = f.readlines()
        for i in range(len(lines) - 1):
            print(" " + str(i) + ".", end=' ')
            for j in range(0, 2, 1):
                print(lines[j],end=' ')
            print("")
            
def create(FilePath):
    with open(FilePath, 'a') as f:
        path = input("Write a Path:")
        pathCpy = path
        path = path.split("/")
        f.write("[" + path[len(path) - 1] + "]\n")
        f.write("path="+str(pathCpy))
def delete(FilePath):
    name = input("Project Name:")
    with open(FilePath, "+w") as f:
        lines = f.readlines()
        linesCpy = f.readlines()
        for lineIdx in range(len(lines)):
            if lines[lineIdx] == name:
                linesCpy.pop(lineIdx + 1)
                linesCpy.remove("[" + name + "]")
def main(FilePath):
    commands = {"help" : help, "show" : show, "create": create, "delete":delete, "help":help}

    try:
        command = argv[1]
        commands[command](FilePath)
    except IndexError:
        help(FilePath)
    except KeyError:
        help(FilePath)
