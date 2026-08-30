namespace MyApp.Services;

public abstract class BaseService
{
    protected const int MaxRetries = 3;

    protected readonly string _serviceName;

    protected BaseService(string serviceName)
    {
        _serviceName = serviceName;
    }

    public virtual string Process(string input)
    {
        return input.Trim();
    }
}