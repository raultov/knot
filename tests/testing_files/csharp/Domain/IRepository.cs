namespace MyApp.Domain;

/// <summary>
/// Generic persistence abstraction for aggregate roots.
/// </summary>
public interface IRepository<T>
{
    /// <summary>Finds an entity by its identifier.</summary>
    Task<T?> FindByIdAsync(int id);

    Task SaveAsync(T entity);
}

public interface IAdminRepository : IRepository<User>
{
    Task<int> PurgeAsync();
}

public interface IUserService
{
    Task<UserDto?> GetUserAsync(int id);
}