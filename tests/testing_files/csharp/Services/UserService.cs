using System;
using System.Threading.Tasks;
using MyApp.Domain;
using MyApp.Infrastructure;

namespace MyApp.Services;

/// <summary>
/// Application service coordinating user retrieval.
/// </summary>
[Obsolete("Use UserServiceV2 instead")]
public class UserService : BaseService, IUserService
{
    private readonly UserRepository _repository;

    public string ServiceLabel { get; private set; }

    public UserService(UserRepository repository) : base("users")
    {
        _repository = repository;
        ServiceLabel = "user-service";
    }

    public async Task<UserDto?> GetUserAsync(int id)
    {
        var user = await _repository.FindByIdAsync(id);
        if (user is null)
        {
            return null;
        }
        return new UserDto(user.Id, user.Name);
    }

    public override string Process(string input)
    {
        return base.Process(input).ToUpperInvariant();
    }
}